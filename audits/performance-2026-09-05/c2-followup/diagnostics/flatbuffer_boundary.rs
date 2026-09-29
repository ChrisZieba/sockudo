use surrealdb_types::{Bytes, SurrealValue, Value};
fn main() {
    for n in [249_521, 250_033] {
        let legacy = vec![vec![65u8; n], vec![65u8; n]].into_value();
        let wire = surrealdb_types::encode(&legacy).unwrap();
        let old = flatbuffers::root::<surrealdb_protocol::fb::v1::Value>(&wire);
        println!(
            "legacy_elements_per_array={n}, encoded_bytes={}, old_verifier={:?}, current_decode={}",
            wire.len(),
            old.map(|_| ()),
            surrealdb_types::decode::<Value>(&wire).is_ok()
        );
        let native = vec![Bytes::from(vec![65u8; n]), Bytes::from(vec![65u8; n])].into_value();
        let wire = surrealdb_types::encode(&native).unwrap();
        println!(
            "native_bytes_per_value={n}, encoded_bytes={}, old_verifier={:?}",
            wire.len(),
            flatbuffers::root::<surrealdb_protocol::fb::v1::Value>(&wire).map(|_| ())
        );
    }
}
