DEFINE FUNCTION IF NOT EXISTS fn::__KEY_FN__($parts: array<string>) {
    RETURN array::join($parts.map(|$p| string::concat(<string>bytes::len(<bytes>$p), ':', $p, '|')), '');
};
DEFINE FUNCTION IF NOT EXISTS fn::__MUTATE_FN__($a: array) {
    LET $stream_id = fn::__KEY_FN__([$a[0], $a[1]]);
    LET $message_id = fn::__KEY_FN__([$a[0], $a[1], $a[2]]);
    LET $entry_id = fn::__KEY_FN__([$a[0], $a[1], $a[2], $a[3]]);
    LET $payload = IF $a[18] { $a[5] } ELSE { <array<int>>$a[5] };
    LET $run = $a[15];
    LET $format_write = UPDATE ONLY __FORMAT__:format SET fence += 1 WHERE epoch = $a[13] AND enabled = $a[14] RETURN AFTER;
    IF $format_write = NONE { THROW 'version_conflict'; };
    LET $stream_write = UPDATE ONLY type::record('__STREAMS__', $stream_id) SET next_delivery_serial = $a[4] + 1, open_stream_count = $a[10], newest_delivery_serial = $a[4], updated_at_ms = $a[6] WHERE next_delivery_serial = $a[8] AND open_stream_count = $a[9] RETURN AFTER;
    IF $stream_write = NONE { THROW 'version_conflict'; };
    LET $message_write = UPDATE ONLY type::record('__MESSAGES__', $message_id) SET latest_version_serial = $a[3], latest_entry_key = $entry_id, latest_payload_bytes = $payload, append_count = $a[11], is_open_stream = $a[12], updated_at_ms = $a[6], latest_append_run = $run[0], latest_append_len = $run[2], latest_append_head = IF $run = NONE { NONE } ELSE { $a[3] }, latest_append_pinned = $run[5], latest_append_generation = $run[1] WHERE latest_version_serial = $a[7] RETURN AFTER;
    IF $message_write = NONE { THROW 'version_conflict'; };
    CREATE ONLY type::record('__ENTRIES__', $entry_id) CONTENT { app_id: $a[0], channel: $a[1], message_serial: $a[2], version_serial: $a[3], delivery_serial: $a[4], payload_bytes: $payload, created_at_ms: $a[6] };
    IF $run != NONE {
        LET $run_id = fn::__KEY_FN__([$a[0], $a[1], $a[2], $run[0], $run[1]]);
        IF $run[3] = 1 {
            UPSERT ONLY type::record('__RUNS__', $run_id) CONTENT { app_id: $a[0], channel: $a[1], message_serial: $a[2], run_version_serial: $run[0], head_version_serial: $a[3], data: '', generation: $run[1], data_len: $run[2], pinned: $run[5], updated_at_ms: $a[6] };
        } ELSE {
            LET $run_write = UPDATE ONLY type::record('__RUNS__', $run_id) SET head_version_serial = $a[3], data_len = $run[2], pinned = pinned OR $run[5], updated_at_ms = $a[6] WHERE head_version_serial = $a[7] AND data_len = $run[4] AND generation = $run[1] RETURN AFTER;
            IF $run_write = NONE { THROW 'version_conflict'; };
        };
        FOR $chunk IN $a[16] {
            LET $chunk_id = fn::__KEY_FN__([$a[0], $a[1], $a[2], $run[1], <string>($chunk[0])]);
            UPSERT type::record('__CHUNKS__', $chunk_id) SET run_id = $run_id, generation = $run[1], chunk_index = $chunk[0], data_bytes = $chunk[1];
        };
    };
    IF $a[17] != NONE {
        LET $receipt_id = fn::__KEY_FN__([$a[0], $a[1], $a[17][0]]);
        CREATE ONLY type::record('__RECEIPTS__', $receipt_id) CONTENT { operation_fingerprint: $a[17][1], payload_bytes: $payload, created_at_ms: $a[6] };
    };
    RETURN true;
};
