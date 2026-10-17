//! Private, bounded native runs for complete sorts exceeding in-memory slots.
//! Rows remain owned by the caller; temporary files contain evaluated keys and
//! row positions only. No store writes, expression re-evaluation, or truncation.
use super::{Bindings, OrderExpression, TemporalContext, Value, check_deadline};
use crate::{
    error::{Error, Result},
    store::Store,
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
};

// Quarter of the existing key budget leaves room for encoding and two merge
// heads. Limits are independent of the upstream materialized relation.
const RUN_BYTES: usize = super::MAX_KEY_BYTES / 4;
const RUN_ENTRIES: usize = super::MAX_KEY_ENTRIES / 4;
const RECORD_BYTES: usize = RUN_BYTES / 2;
const MAX_RUNS: usize = 128;
const WRITE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
enum Atom {
    Ref(i64),
    Str(String),
    Int(i64),
    Float(u64),
    Bool(bool),
    Bytes(Vec<u8>),
    Lang(String, String),
    Typed(String, String),
}
impl Atom {
    fn from_value(value: Value) -> Self {
        match value {
            Value::Ref(x) => Self::Ref(x),
            Value::Str(x) => Self::Str(x),
            Value::Int(x) => Self::Int(x),
            Value::Float(x) => Self::Float(x.to_bits()),
            Value::Bool(x) => Self::Bool(x),
            Value::Bytes(x) => Self::Bytes(x),
            Value::Lang { lexical, lang } => Self::Lang(lexical, lang),
            Value::Typed { lexical, datatype } => Self::Typed(lexical, datatype),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    index: usize,
    keys: Vec<(Option<Atom>, Option<String>)>,
}
impl Record {
    fn retained_bytes(&self) -> usize {
        let mut bytes = (2 * std::mem::size_of::<Self>()).saturating_add(
            self.keys
                .capacity()
                .saturating_mul(std::mem::size_of::<(Option<Atom>, Option<String>)>()),
        );
        for (atom, iri) in &self.keys {
            bytes = bytes
                .saturating_add(match atom {
                    Some(Atom::Str(s)) => s.capacity(),
                    Some(Atom::Bytes(s)) => s.capacity(),
                    Some(Atom::Lang(a, b) | Atom::Typed(a, b)) => {
                        a.capacity().saturating_add(b.capacity())
                    }
                    _ => 0,
                })
                .saturating_add(iri.as_ref().map_or(0, String::capacity));
        }
        bytes
    }

    fn compare(&self, other: &Self, expression: &[OrderExpression]) -> Ordering {
        for ((order, (a, ia)), (b, ib)) in expression.iter().zip(&self.keys).zip(&other.keys) {
            let cmp = match (ia, ib) {
                (Some(a), Some(b)) => a.cmp(b),
                _ => {
                    // Keep the existing heterogeneous/typed numeric ordering.
                    // Wire values are borrowed via a lossless view below.
                    compare_atoms(a.as_ref(), b.as_ref())
                }
            };
            let cmp = if matches!(order, OrderExpression::Asc(_)) {
                cmp
            } else {
                cmp.reverse()
            };
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        self.index.cmp(&other.index)
    }
}

fn compare_atoms(a: Option<&Atom>, b: Option<&Atom>) -> Ordering {
    // Conversion uses owned payloads only for a pair of bounded records; it does
    // not evaluate expressions or resolve dictionary references again.
    fn value(atom: &Atom) -> Value {
        match atom {
            Atom::Ref(x) => Value::Ref(*x),
            Atom::Str(x) => Value::Str(x.clone()),
            Atom::Int(x) => Value::Int(*x),
            Atom::Float(x) => Value::Float(f64::from_bits(*x)),
            Atom::Bool(x) => Value::Bool(*x),
            Atom::Bytes(x) => Value::Bytes(x.clone()),
            Atom::Lang(x, y) => Value::Lang {
                lexical: x.clone(),
                lang: y.clone(),
            },
            Atom::Typed(x, y) => Value::Typed {
                lexical: x.clone(),
                datatype: y.clone(),
            },
        }
    }
    super::super::aggregate::compare_option_values(&a.map(value), &b.map(value))
}

fn refused() -> Error {
    Error::InvalidValue("ORDER BY retained key budget or native spill quota exceeded".into())
}
fn io_error(error: impl std::fmt::Display) -> Error {
    Error::InvalidValue(format!("ORDER BY private spill failed: {error}"))
}

fn write_record(file: &mut impl Write, record: &Record, written: &mut u64) -> Result<()> {
    if record.retained_bytes() > RECORD_BYTES {
        return Err(refused());
    }
    // A capped writer prevents serde from growing an unbounded encoded buffer.
    struct Capped(Vec<u8>);
    impl Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > RECORD_BYTES {
                return Err(std::io::Error::other("record budget"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = Capped(Vec::new());
    serde_json::to_writer(&mut encoded, record).map_err(|_| refused())?;
    *written = written
        .checked_add(encoded.0.len() as u64 + 4)
        .ok_or_else(refused)?;
    if *written > WRITE_BYTES {
        return Err(refused());
    }
    file.write_all(&(encoded.0.len() as u32).to_le_bytes())
        .map_err(io_error)?;
    file.write_all(&encoded.0).map_err(io_error)
}

fn read_record(file: &mut impl Read) -> Result<Option<Record>> {
    let mut size = [0; 4];
    if file.read(&mut size[..1]).map_err(io_error)? == 0 {
        return Ok(None);
    }
    file.read_exact(&mut size[1..]).map_err(io_error)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > RECORD_BYTES {
        return Err(refused());
    }
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(io_error)?;
    let record: Record = serde_json::from_slice(&bytes).map_err(io_error)?;
    if record.retained_bytes() > RECORD_BYTES {
        return Err(refused());
    }
    Ok(Some(record))
}

fn flush_run(
    records: &mut Vec<Record>,
    expression: &[OrderExpression],
    ctx: &TemporalContext,
    written: &mut u64,
) -> Result<File> {
    check_deadline(ctx)?;
    // As in the in-memory path, do not hand a partial SPARQL ordering to
    // slice::sort_by (which may panic on incomparable heterogeneous values).
    let mut order: Vec<_> = (0..records.len()).collect();
    let mut scratch = vec![0; order.len()];
    let mut width = 1;
    while width < order.len() {
        for start in (0..order.len()).step_by(width * 2) {
            let mid = (start + width).min(order.len());
            let end = (start + width * 2).min(order.len());
            let (mut left, mut right) = (start, mid);
            for slot in &mut scratch[start..end] {
                check_deadline(ctx)?;
                if left < mid
                    && (right == end
                        || records[order[left]].compare(&records[order[right]], expression)
                            != Ordering::Greater)
                {
                    *slot = order[left];
                    left += 1;
                } else {
                    *slot = order[right];
                    right += 1;
                }
            }
        }
        std::mem::swap(&mut order, &mut scratch);
        width *= 2;
    }
    check_deadline(ctx)?;
    // Anonymous temporary files are removed on every success/error/cancellation
    // path by their owning File handles, including process termination.
    let mut file = BufWriter::with_capacity(32 * 1024, tempfile::tempfile().map_err(io_error)?);
    let mut pending: Vec<_> = records.drain(..).map(Some).collect();
    for index in order {
        check_deadline(ctx)?;
        let record = pending[index].take().ok_or_else(refused)?;
        write_record(&mut file, &record, written)?;
    }
    let mut file = file.into_inner().map_err(io_error)?;
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    Ok(file)
}

fn merge(
    left: File,
    right: File,
    expression: &[OrderExpression],
    ctx: &TemporalContext,
    written: &mut u64,
) -> Result<File> {
    let mut left = BufReader::with_capacity(32 * 1024, left);
    let mut right = BufReader::with_capacity(32 * 1024, right);
    let mut output = BufWriter::with_capacity(32 * 1024, tempfile::tempfile().map_err(io_error)?);
    let (mut a, mut b) = (read_record(&mut left)?, read_record(&mut right)?);
    while a.is_some() || b.is_some() {
        check_deadline(ctx)?;
        let take_left = match (&a, &b) {
            (Some(a), Some(b)) => a.compare(b, expression) != Ordering::Greater,
            (Some(_), None) => true,
            _ => false,
        };
        let record = if take_left { a.take() } else { b.take() }.ok_or_else(refused)?;
        write_record(&mut output, &record, written)?;
        if take_left {
            a = read_record(&mut left)?;
        } else {
            b = read_record(&mut right)?;
        }
    }
    let mut output = output.into_inner().map_err(io_error)?;
    output.seek(SeekFrom::Start(0)).map_err(io_error)?;
    Ok(output)
}

pub(super) fn sort(
    store: &Store,
    rows: Vec<Bindings>,
    expression: &[OrderExpression],
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    check_deadline(ctx)?;
    if expression.len() > RUN_ENTRIES {
        return Err(refused());
    }
    let (mut runs, mut records) = (Vec::new(), Vec::new());
    let (mut bytes, mut entries, mut written) = (0usize, 0usize, 0u64);
    for (index, row) in rows.iter().enumerate() {
        check_deadline(ctx)?;
        let mut keys = Vec::with_capacity(expression.len());
        let mut cost = std::mem::size_of::<Record>();
        for order in expression {
            check_deadline(ctx)?;
            let (OrderExpression::Asc(expr) | OrderExpression::Desc(expr)) = order;
            let value = super::super::filter::eval_expr(store, expr, row);
            check_deadline(ctx)?;
            cost = cost
                .checked_add(
                    super::payload_bytes(value.as_ref())
                        .saturating_add(std::mem::size_of::<(Option<Atom>, Option<String>)>()),
                )
                .ok_or_else(refused)?;
            let iri = match value.as_ref() {
                Some(Value::Ref(id)) => Some(store.resolve(*id)?),
                _ => None,
            };
            cost = cost
                .checked_add(iri.as_ref().map_or(0, String::capacity))
                .ok_or_else(refused)?;
            if cost > RECORD_BYTES || expression.len() > RUN_ENTRIES {
                return Err(refused());
            }
            keys.push((value.map(Atom::from_value), iri));
        }
        let record = Record { index, keys };
        let cost = record.retained_bytes();
        if cost > RECORD_BYTES {
            return Err(refused());
        }
        if bytes.saturating_add(cost) > RUN_BYTES || entries + record.keys.len() > RUN_ENTRIES {
            runs.push(flush_run(&mut records, expression, ctx, &mut written)?);
            if runs.len() >= MAX_RUNS {
                return Err(refused());
            }
            bytes = 0;
            entries = 0;
        }
        bytes += cost;
        entries += record.keys.len();
        records.push(record);
    }
    if !records.is_empty() {
        runs.push(flush_run(&mut records, expression, ctx, &mut written)?);
    }
    while runs.len() > 1 {
        let mut next = Vec::new();
        let mut iter = runs.into_iter();
        while let Some(left) = iter.next() {
            next.push(match iter.next() {
                Some(right) => merge(left, right, expression, ctx, &mut written)?,
                None => left,
            });
        }
        runs = next;
    }
    let mut rows: Vec<_> = rows.into_iter().map(Some).collect();
    let mut output = Vec::with_capacity(rows.len());
    if let Some(run) = runs.pop() {
        let mut run = BufReader::with_capacity(32 * 1024, run);
        while let Some(record) = read_record(&mut run)? {
            check_deadline(ctx)?;
            output.push(
                rows.get_mut(record.index)
                    .and_then(Option::take)
                    .ok_or_else(refused)?,
            );
        }
    }
    if output.len() != rows.len() {
        return Err(refused());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_preserves_float_bits_and_long_datatype_without_store_codec_truncation() {
        let record = Record {
            index: 42,
            keys: vec![
                (Some(Atom::Float(0x7ff8_0000_0000_0001)), None),
                (Some(Atom::Float((-0.0f64).to_bits())), None),
                (Some(Atom::Typed("v".into(), "d".repeat(70_000))), None),
                (Some(Atom::Lang("hello@en".into(), "en-GB".into())), None),
                (Some(Atom::Bytes(vec![0, 255])), None),
                (None, None),
            ],
        };
        let mut file = tempfile::tempfile().unwrap();
        write_record(&mut file, &record, &mut 0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let decoded = read_record(&mut file).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::to_value(decoded).unwrap()
        );
        assert!(read_record(&mut file).unwrap().is_none());
    }

    #[test]
    fn quota_and_encoded_record_limits_refuse_without_partial_output() {
        let record = Record {
            index: 0,
            keys: vec![(Some(Atom::Int(1)), None)],
        };
        let mut file = tempfile::tempfile().unwrap();
        let mut exhausted = WRITE_BYTES;
        assert!(write_record(&mut file, &record, &mut exhausted).is_err());
        assert_eq!(file.metadata().unwrap().len(), 0);
        let oversized = Record {
            index: 0,
            keys: vec![(Some(Atom::Str("x".repeat(RECORD_BYTES))), None)],
        };
        assert!(write_record(&mut file, &oversized, &mut 0).is_err());
        assert_eq!(file.metadata().unwrap().len(), 0);
    }

    #[test]
    fn expired_spill_refuses_before_creating_runs() {
        let store = Store::open_in_memory().unwrap();
        let ctx = TemporalContext {
            deadline: Some(crate::time::Deadline::after_millis(0)),
            ..Default::default()
        };
        assert!(matches!(
            sort(&store, vec![Bindings::new()], &[], &ctx),
            Err(Error::QueryTimeout { .. })
        ));
    }
}
