fn main() {
    println!("hello from {{ project_name }}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {
        assert!(true);
    }
}
