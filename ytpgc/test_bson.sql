-- SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
-- drop EXTENSION IF EXISTS yt_cluster;
-- CREATE EXTENSION yt_cluster;

-- test_bson.sql - functional tests for yt_diff_apply_bson(), yt_bson_to_json()
-- and yt_bson_from_json(): a BSON document of DocumentDB / MongoDB patched.
--
-- The documents and patches are diff-doc-rs's (feature bson): a DocumentDB order,
--   base = {_id: ObjectId, sku: "A-1", qty: int64 5, price: decimal 9.90,
--           at: date 2024-05-01T10:00:00Z, tags: ["new"], w: 1.5}
--   A    = price 8.50 and at 2024-05-02 (the hunks diff_doc::diff::Mismatch made)
--   B    = qty 7 and tags + "sale"
-- and the bytes of base and of base + A + B are diff-doc-rs's to_bson output.
--
-- Run with:  psql -d <db> -f test_bson.sql
-- Each SELECT should return 't' (true) for a passing assertion.

\echo '=== yt_diff_apply_bson / yt_bson_to_json / yt_bson_from_json tests ==='

-- the query raises an error whose message contains `msg`
CREATE FUNCTION pg_temp.raises(q text, msg text) RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    EXECUTE q;
    RETURN false;
EXCEPTION WHEN others THEN
    RETURN position(msg IN SQLERRM) > 0;
END $$;

CREATE TEMP TABLE bson_case (name text PRIMARY KEY, doc bytea);
INSERT INTO bson_case VALUES
    -- diff-doc-rs to_bson of base
    ('base', '\x74000000075f69640065f1a2b3c4d5e6f708192a3b0961740000c198338f01000013707269636500de030000000000000000000000003c301271747900050000000000000002736b750004000000412d310004746167730010000000023000040000006e65770000017700000000000000f83f00'),
    -- diff-doc-rs to_bson of base + A + B
    ('result', '\x80000000075f69640065f1a2b3c4d5e6f708192a3b09617400001dbf388f0100001370726963650052030000000000000000000000003c301271747900070000000000000002736b750004000000412d31000474616773001c000000023000040000006e6577000231000500000073616c650000017700000000000000f83f00');

CREATE TEMP TABLE bson_patch (name text PRIMARY KEY, diff bytea);
INSERT INTO bson_patch VALUES
    ('a', yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"at"},{"name":"$date"}],"value":{"update":{"string":"2024-05-02T10:00:00Z"}}},
        {"path":[{"name":"price"},{"name":"$numberDecimal"}],"value":{"update":{"numeric":"8.50"}}}
    ]}}'::jsonb)),
    ('b', yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"qty"},{"name":"$numberLong"}],"value":{"update":{"numeric":"7"}}},
        {"path":[{"name":"tags"},{"idx":1}],"value":{"insert":{"string":"sale"}}}
    ]}}'::jsonb));

-- ─────────────────────────────────────────────────────────────────────────────
-- The mapping: BSON as extended JSON, and back to the same bytes
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- BSON as extended JSON ---'

SELECT yt_bson_to_json(doc) = '{"_id": {"$oid": "65f1a2b3c4d5e6f708192a3b"}, "sku": "A-1",
        "qty": {"$numberLong": 5}, "price": {"$numberDecimal": 9.90},
        "at": {"$date": "2024-05-01T10:00:00Z"}, "tags": ["new"], "w": 1.5}'::jsonb
    AS bson_to_json
FROM bson_case WHERE name = 'base';

-- the same document from its extended JSON: diff-doc-rs's bytes, _id first
SELECT yt_bson_from_json('{"w": 1.5, "tags": ["new"], "sku": "A-1",
        "qty": {"$numberLong": "5"}, "price": {"$numberDecimal": "9.90"},
        "at": {"$date": "2024-05-01T10:00:00Z"}, "_id": {"$oid": "65f1a2b3c4d5e6f708192a3b"}}'::jsonb) = doc
    AS bson_from_json_same_bytes
FROM bson_case WHERE name = 'base';

-- every case, one row: one `t` per assertion
SELECT bool_and(yt_bson_from_json(yt_bson_to_json(doc)) = doc) AS bson_json_round_trip
FROM bson_case;

-- each type reads back the type it was: int32, int64, double, decimal, date
SELECT yt_bson_to_json(yt_bson_from_json('{"i": 7, "l": {"$numberLong": "7"}, "d": 7.0,
        "m": {"$numberDecimal": "7"}, "t": {"$date": {"$numberLong": "-1"}}}'::jsonb))
     = '{"i": 7, "l": {"$numberLong": 7}, "d": 7.0, "m": {"$numberDecimal": 7},
         "t": {"$date": {"$numberLong": -1}}}'::jsonb
    AS bson_types_kept;

SELECT get_byte(yt_bson_from_json('{"i": 7}'::jsonb), 4) = 16          -- int32
   AND get_byte(yt_bson_from_json('{"i": 7.0}'::jsonb), 4) = 1         -- double
   AND get_byte(yt_bson_from_json('{"i": 5000000000}'::jsonb), 4) = 18 -- int64
   AND get_byte(yt_bson_from_json('{"i": {"$numberLong": "7"}}'::jsonb), 4) = 18
    AS bson_number_types;

-- ─────────────────────────────────────────────────────────────────────────────
-- Patches: A and B disjoint, in either order diff-doc-rs's bytes
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- BSON patches A and B ---'

SELECT yt_bson_to_json(yt_diff_apply_bson(c.doc, p.diff))
     = '{"_id": {"$oid": "65f1a2b3c4d5e6f708192a3b"}, "sku": "A-1",
         "qty": {"$numberLong": 5}, "price": {"$numberDecimal": 8.50},
         "at": {"$date": "2024-05-02T10:00:00Z"}, "tags": ["new"], "w": 1.5}'::jsonb
    AS bson_apply_a
FROM bson_case c, bson_patch p WHERE c.name = 'base' AND p.name = 'a';

SELECT yt_bson_to_json(yt_diff_apply_bson(c.doc, p.diff))
     = '{"_id": {"$oid": "65f1a2b3c4d5e6f708192a3b"}, "sku": "A-1",
         "qty": {"$numberLong": 7}, "price": {"$numberDecimal": 9.90},
         "at": {"$date": "2024-05-01T10:00:00Z"}, "tags": ["new", "sale"], "w": 1.5}'::jsonb
    AS bson_apply_b
FROM bson_case c, bson_patch p WHERE c.name = 'base' AND p.name = 'b';

SELECT yt_diff_apply_bson(yt_diff_apply_bson(c.doc, a.diff), b.diff) = r.doc AS bson_base_a_b_rust_bytes
FROM bson_case c, bson_case r, bson_patch a, bson_patch b
WHERE c.name = 'base' AND r.name = 'result' AND a.name = 'a' AND b.name = 'b';

SELECT yt_diff_apply_bson(yt_diff_apply_bson(c.doc, b.diff), a.diff) = r.doc AS bson_base_b_a_rust_bytes
FROM bson_case c, bson_case r, bson_patch a, bson_patch b
WHERE c.name = 'base' AND r.name = 'result' AND a.name = 'a' AND b.name = 'b';

-- a document stored as BSON, patched in place by an UPDATE
CREATE TEMP TABLE orders (id int PRIMARY KEY, doc bytea);
INSERT INTO orders SELECT 1, doc FROM bson_case WHERE name = 'base';
UPDATE orders SET doc = yt_diff_apply_bson(doc, (SELECT diff FROM bson_patch WHERE name = 'b'));
UPDATE orders SET doc = yt_diff_apply_bson(doc, (SELECT diff FROM bson_patch WHERE name = 'a'));
SELECT o.doc = r.doc AS bson_update_in_place
FROM orders o, bson_case r WHERE r.name = 'result';

-- the patched document: _id first (ObjectId 0x07, "_id"), int64 kept (0x12)
SELECT get_byte(doc, 4) = 7 AND substring(doc FROM 6 FOR 4) = '\x5f696400'::bytea
   AND position('\x127174790007000000'::bytea IN doc) > 0
    AS bson_id_first_int64_kept
FROM orders;

-- a key added by a patch: sorted into place, the same bytes whatever the order of adds
SELECT yt_diff_apply_bson(yt_diff_apply_bson(c.doc,
           yt_diff_encode('{"doc":{"hunks":[{"path":[{"name":"b"}],"value":{"insert":{"numeric":"1"}}}]}}'::jsonb)),
           yt_diff_encode('{"doc":{"hunks":[{"path":[{"name":"a"}],"value":{"insert":{"numeric":"2"}}}]}}'::jsonb))
     = yt_diff_apply_bson(yt_diff_apply_bson(c.doc,
           yt_diff_encode('{"doc":{"hunks":[{"path":[{"name":"a"}],"value":{"insert":{"numeric":"2"}}}]}}'::jsonb)),
           yt_diff_encode('{"doc":{"hunks":[{"path":[{"name":"b"}],"value":{"insert":{"numeric":"1"}}}]}}'::jsonb))
    AS bson_adds_commute_bytes
FROM bson_case c WHERE c.name = 'base';

-- ─────────────────────────────────────────────────────────────────────────────
-- Errors
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- BSON errors ---'

SELECT pg_temp.raises($$SELECT yt_bson_to_json('\x0500'::bytea)$$, 'BSON: ')
    AS bson_err_truncated;

SELECT pg_temp.raises($$SELECT yt_bson_to_json('\x0800000020610000'::bytea)$$, 'unknown type 0x20')
    AS bson_err_unknown_type;

SELECT pg_temp.raises($$SELECT yt_bson_from_json('[1]'::jsonb)$$, 'DD-E501')
    AS bson_err_not_a_document;

SELECT pg_temp.raises($$SELECT yt_bson_from_json('{"n": 18446744073709551616}'::jsonb)$$,
                      'DD-E502 the number 18446744073709551616 at /n')
    AS bson_err_past_int64;

SELECT pg_temp.raises($$SELECT yt_bson_from_json('{"_id": {"$oid": "nope"}}'::jsonb)$$,
                      'not an ObjectId at /_id')
    AS bson_err_bad_oid;

-- a patch that leaves no valid date: refused, not written
SELECT pg_temp.raises(format($$SELECT yt_diff_apply_bson(%L::bytea, yt_diff_encode(%L::jsonb))$$, doc,
        '{"doc":{"hunks":[{"path":[{"name":"at"},{"name":"$date"}],"value":{"update":{"string":"yesterday"}}}]}}'),
        'not a $date at /at')
    AS bson_err_patch_bad_date
FROM bson_case WHERE name = 'base';

-- a patch on a path the document does not have
SELECT pg_temp.raises(format($$SELECT yt_diff_apply_bson(%L::bytea, yt_diff_encode(%L::jsonb))$$, doc,
        '{"doc":{"hunks":[{"path":[{"name":"none"},{"name":"x"}],"value":{"update":{"numeric":"1"}}}]}}'),
        'yt_diff_apply_bson: ')
    AS bson_err_patch_no_path
FROM bson_case WHERE name = 'base';

\echo '=== done ==='
