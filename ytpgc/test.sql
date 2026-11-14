-- SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
-- drop EXTENSION IF EXISTS yt_cluster;
-- CREATE EXTENSION yt_cluster;

-- test.sql - functional tests for yt_diff_encode() and yt_diff_apply()
--
-- The three JSON test cases of diff-doc-rs:
--   case1 - object field update
--   case2 - array element update
--   case3 - nested field update + element removal
--
-- Run with:  psql -d <db> -f test.sql
-- Each SELECT should return 't' (true) for a passing assertion.

\echo '=== yt_diff_encode / yt_diff_apply tests ==='

-- ─────────────────────────────────────────────────────────────────────────────
-- Helpers
-- ─────────────────────────────────────────────────────────────────────────────

-- Round-trip sanity: encoding a no-op diff returns non-empty bytea
SELECT length(yt_diff_encode('{"doc":{"hunks":[]}}'::jsonb)) > 0
    AS encode_nonempty;

-- ─────────────────────────────────────────────────────────────────────────────
-- Case 1 - object field updates 
--
--   base   = {"base":"json file case1","line":"base","another":"line"}
--   a      = base + update "line"    → "changed"
--   b      = base + update "another" → "changed"
--   result = base + diff_a + diff_b  (commutative)
--          = {"another":"changed","base":"json file case1","line":"changed"}
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- case 1: object field updates ---'

-- diff_a encodes: update path=["line"] → string "changed"
SELECT yt_diff_apply(
    '{"base":"json file case1","line":"base","another":"line"}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"line"}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '{"another":"line","base":"json file case1","line":"changed"}'::jsonb
    AS case1_apply_diff_a;

-- diff_b encodes: update path=["another"] → string "changed"
SELECT yt_diff_apply(
    '{"base":"json file case1","line":"base","another":"line"}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"another"}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '{"another":"changed","base":"json file case1","line":"base"}'::jsonb
    AS case1_apply_diff_b;

-- Apply diff_a then diff_b → result
SELECT yt_diff_apply(
    yt_diff_apply(
        '{"base":"json file case1","line":"base","another":"line"}'::jsonb,
        yt_diff_encode('{"doc":{"hunks":[
            {"path":[{"name":"line"}],"value":{"update":{"string":"changed"}}}
        ]}}'::jsonb)
    ),
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"another"}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '{"another":"changed","base":"json file case1","line":"changed"}'::jsonb
    AS case1_result_a_then_b;

-- Apply diff_b then diff_a → same result (commutative)
SELECT yt_diff_apply(
    yt_diff_apply(
        '{"base":"json file case1","line":"base","another":"line"}'::jsonb,
        yt_diff_encode('{"doc":{"hunks":[
            {"path":[{"name":"another"}],"value":{"update":{"string":"changed"}}}
        ]}}'::jsonb)
    ),
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"line"}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '{"another":"changed","base":"json file case1","line":"changed"}'::jsonb
    AS case1_result_b_then_a;

-- ─────────────────────────────────────────────────────────────────────────────
-- Case 2 - array element updates 
--
--   base   = ["json file case2","base","line"]
--   a      = base + update idx=1 → "changed"
--   b      = base + update idx=2 → "changed"
--   result = ["json file case2","changed","changed"]
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- case 2: array element updates ---'

SELECT yt_diff_apply(
    '["json file case2","base","line"]'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":1}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '["json file case2","changed","line"]'::jsonb
    AS case2_apply_diff_a;

SELECT yt_diff_apply(
    '["json file case2","base","line"]'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":2}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '["json file case2","base","changed"]'::jsonb
    AS case2_apply_diff_b;

-- Combined (a then b)
SELECT yt_diff_apply(
    yt_diff_apply(
        '["json file case2","base","line"]'::jsonb,
        yt_diff_encode('{"doc":{"hunks":[
            {"path":[{"idx":1}],"value":{"update":{"string":"changed"}}}
        ]}}'::jsonb)
    ),
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":2}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '["json file case2","changed","changed"]'::jsonb
    AS case2_result_a_then_b;

-- Combined (b then a) - commutative
SELECT yt_diff_apply(
    yt_diff_apply(
        '["json file case2","base","line"]'::jsonb,
        yt_diff_encode('{"doc":{"hunks":[
            {"path":[{"idx":2}],"value":{"update":{"string":"changed"}}}
        ]}}'::jsonb)
    ),
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":1}],"value":{"update":{"string":"changed"}}}
    ]}}'::jsonb)
) = '["json file case2","changed","changed"]'::jsonb
    AS case2_result_b_then_a;

-- ─────────────────────────────────────────────────────────────────────────────
-- Case 3 - mixed nested update + removal 
--
--   base   = [{"name":"json file case3"},{"name":"base"},{"name":"line"},
--              {"name":"to delete"},{"name":"the end"}]
--   diff_a = update [1].name → "base changed"                   (1 hunk)
--   diff_b = update [2].name → "changed" + remove [3]           (2 hunks)
--   result = [{"name":"json file case3"},{"name":"base changed"},
--              {"name":"changed"},{"name":"the end"}]
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- case 3: nested field update + element removal ---'

-- diff_a only
SELECT yt_diff_apply(
    '[{"name":"json file case3"},{"name":"base"},{"name":"line"},
      {"name":"to delete"},{"name":"the end"}]'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":1},{"name":"name"}],
         "value":{"update":{"string":"base changed"}}}
    ]}}'::jsonb)
) = '[{"name":"json file case3"},{"name":"base changed"},{"name":"line"},
     {"name":"to delete"},{"name":"the end"}]'::jsonb
    AS case3_apply_diff_a;

-- diff_b only (update idx 2 name, remove idx 3)
SELECT yt_diff_apply(
    '[{"name":"json file case3"},{"name":"base"},{"name":"line"},
      {"name":"to delete"},{"name":"the end"}]'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":2},{"name":"name"}],
         "value":{"update":{"string":"changed"}}},
        {"path":[{"idx":3}],
         "value":{"remove":true}}
    ]}}'::jsonb)
) = '[{"name":"json file case3"},{"name":"base"},{"name":"changed"},
     {"name":"the end"}]'::jsonb
    AS case3_apply_diff_b;

-- Apply diff_a then diff_b → result
SELECT yt_diff_apply(
    yt_diff_apply(
        '[{"name":"json file case3"},{"name":"base"},{"name":"line"},
          {"name":"to delete"},{"name":"the end"}]'::jsonb,
        yt_diff_encode('{"doc":{"hunks":[
            {"path":[{"idx":1},{"name":"name"}],
             "value":{"update":{"string":"base changed"}}}
        ]}}'::jsonb)
    ),
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":2},{"name":"name"}],
         "value":{"update":{"string":"changed"}}},
        {"path":[{"idx":3}],
         "value":{"remove":true}}
    ]}}'::jsonb)
) = '[{"name":"json file case3"},{"name":"base changed"},{"name":"changed"},
     {"name":"the end"}]'::jsonb
    AS case3_result_a_then_b;

-- Apply diff_b then diff_a → same result (commutative)
-- After diff_b, array is 4 elements; [1] is still "base", so diff_a still applies to idx 1.
SELECT yt_diff_apply(
    yt_diff_apply(
        '[{"name":"json file case3"},{"name":"base"},{"name":"line"},
          {"name":"to delete"},{"name":"the end"}]'::jsonb,
        yt_diff_encode('{"doc":{"hunks":[
            {"path":[{"idx":2},{"name":"name"}],
             "value":{"update":{"string":"changed"}}},
            {"path":[{"idx":3}],
             "value":{"remove":true}}
        ]}}'::jsonb)
    ),
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"idx":1},{"name":"name"}],
         "value":{"update":{"string":"base changed"}}}
    ]}}'::jsonb)
) = '[{"name":"json file case3"},{"name":"base changed"},{"name":"changed"},
     {"name":"the end"}]'::jsonb
    AS case3_result_b_then_a;

-- ─────────────────────────────────────────────────────────────────────────────
-- GenericValue type coverage
-- ─────────────────────────────────────────────────────────────────────────────

\echo '--- GenericValue type coverage ---'

-- Insert a numeric value
SELECT yt_diff_apply(
    '{"x":1}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"x"}],"value":{"update":{"numeric":"42"}}}
    ]}}'::jsonb)
) = '{"x":42}'::jsonb
    AS gv_numeric;

-- Insert a boolean value
SELECT yt_diff_apply(
    '{"flag":false}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"flag"}],"value":{"update":{"boolean":true}}}
    ]}}'::jsonb)
) = '{"flag":true}'::jsonb
    AS gv_boolean;

-- Insert a null value
SELECT yt_diff_apply(
    '{"v":"old"}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"v"}],"value":{"update":{"null":true}}}
    ]}}'::jsonb)
) = '{"v":null}'::jsonb
    AS gv_null;

-- Insert a nested object (map) value
SELECT yt_diff_apply(
    '{"nested":{}}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"nested"}],
         "value":{"update":{"map":{"fields":{"a":{"string":"hello"}}}}}}
    ]}}'::jsonb)
) = '{"nested":{"a":"hello"}}'::jsonb
    AS gv_map;

-- Insert an array value
SELECT yt_diff_apply(
    '{"arr":[]}'::jsonb,
    yt_diff_encode('{"doc":{"hunks":[
        {"path":[{"name":"arr"}],
         "value":{"update":{"array":{"items":[
             {"string":"x"},{"numeric":"1"},{"boolean":false}
         ]}}}}
    ]}}'::jsonb)
) = '{"arr":["x",1,false]}'::jsonb
    AS gv_array;

\echo '=== done ==='
