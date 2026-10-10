# Trust-root ceremony: enrol the first human key

Once per store, ever (aegis-kzt0ql.9.4). After this, every further key,
edit or revocation of a human-tier registration is an amendment signed by
an enrolled human key. Nobody can run the bootstrap a second time.

## Before you start

- **Schedule it immediately after the gate is deployed.** Until the
  ceremony runs, anyone who can run `quipu` against the store file could
  bootstrap their own key first. Keep that window short.
- Run it on the quipu host, as the service user, against the live store.
  The server can stay up: SQLite serializes the single write.
- Have the device whose key you are enrolling. Ed25519 keys work today;
  hardware schemes (passkey, security key) need aegis-kzt0ql.9.2 deployed.
- Decide which decision policies this key may sign, in addition to the
  trust-root policy it always gets.

## Steps

1. Check that nothing has been enrolled yet:

       quipu trust-root status --db <store>

   It must print `bootstrapped no`. **If it prints `yes` and you did not
   run the ceremony, stop: that is a compromise. Escalate.**

2. Get the challenge for your key:

       quipu trust-root challenge --verifier stiwi --public-key <hex> --db <store>

   Note the `fingerprint` line.

3. Sign the printed challenge bytes with the device, exactly as printed,
   with no trailing newline. Keep the hex signature.

4. Enrol:

       quipu trust-root bootstrap --verifier stiwi --public-key <hex> \
           --pop-signature <hex> --attests <decision policy> --db <store>

5. **Compare fingerprints.** The `FINGERPRINT` line from step 4 must equal
   the fingerprint your device (or `ssh-keygen -lf` on your public key)
   shows, character for character. **Any mismatch means someone else
   bootstrapped first. Stop, do not use the store for human decisions, and
   escalate.** This step is the only check that the enrolled key is yours;
   proof of possession proves the key, not the person.

6. Record the ceremony on the bead: the time, the registration IRI, and
   the fingerprint you compared. Acknowledge the bootstrap alert.

7. Enrol a second device (recovery) as an amendment signed by the first,
   using `trust_root::digest_after` to compute the digest to sign. With
   only one enrolled device, losing it means no key can amend the
   registry, and the bootstrap cannot be re-run.

## What this does not protect against

Root on the quipu host. Someone who can edit the database file, replace
the binary, or run `import`/`unpack`/`restore` is past this gate. The gate
stops graph writers (anything holding the write bearer), not hosts.
