/* SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent */
/*
 * diff_apply_pg.c - yt_diff_apply() PostgreSQL function
 *
 * Signature: yt_diff_apply(doc jsonb, diff bytea) RETURNS jsonb
 *
 * Decodes a protobuf-encoded DiffDoc::Mismatches (bytea),
 * applies it to a jsonb document using mismatches_apply_mut(),
 * and returns the resulting jsonb.
 *
 * The same for a BSON document (DocumentDB / MongoDB), as bytea, through
 * diff-doc-rs's bson_doc.c - its types beyond JSON's as extended JSON:
 *   yt_diff_apply_bson(doc bytea, diff bytea) RETURNS bytea
 *   yt_bson_to_json(doc bytea) RETURNS jsonb
 *   yt_bson_from_json(doc jsonb) RETURNS bytea
 */

#include "yt_cluster.h"
#include "proto/diff_doc.pb-c.h"
#include "proto/diff_apply.h"
#include "proto/bson_doc.h"

#include "utils/jsonb.h"
#include "utils/builtins.h"

#include <stdlib.h>
#include <string.h>

/* ============================================================================
 * jsonb → GenericValue
 *
 * All allocations use malloc() so that gv_free() (which calls free()) can
 * reclaim everything.  palloc is intentionally avoided here.
 * ============================================================================ */

/* Forward declaration: gv_build is mutually recursive with itself */
static GenericValue *gv_build(JsonbIterator **it,
                               JsonbIteratorToken tok,
                               JsonbValue *jv);

static GenericValue *
gv_from_jsonb(Jsonb *jb)
{
    JsonbIterator     *it  = JsonbIteratorInit(&jb->root);
    JsonbValue         jv;
    JsonbIteratorToken tok = JsonbIteratorNext(&it, &jv, false);

    /*
     * PostgreSQL stores a raw scalar jsonb as a single-element pseudo-array
     * with rawScalar=true.  Unwrap it to a plain scalar GenericValue.
     */
    if (tok == WJB_BEGIN_ARRAY && jv.val.array.rawScalar)
    {
        JsonbValue sv;
        JsonbIteratorNext(&it, &sv, false); /* WJB_ELEM - the scalar */
        return gv_build(&it, WJB_ELEM, &sv);
    }
    return gv_build(&it, tok, &jv);
}

static GenericValue *
gv_build(JsonbIterator **it, JsonbIteratorToken tok, JsonbValue *jv)
{
    GenericValue *result = (GenericValue *) calloc(1, sizeof(GenericValue));

    if (!result)
        return NULL;

    switch (tok)
    {
        /* ── Scalar (array element or object value) ─────────────────── */
        case WJB_VALUE:
        case WJB_ELEM:
            switch (jv->type)
            {
                case jbvNull:
                    result->kind = GV_NULL;
                    break;

                case jbvBool:
                    result->kind    = GV_BOOL;
                    result->boolean = jv->val.boolean ? 1 : 0;
                    break;

                case jbvNumeric:
                {
                    /* Convert Numeric to its decimal string representation */
                    char *ns = DatumGetCString(
                        DirectFunctionCall1(numeric_out,
                                            NumericGetDatum(jv->val.numeric)));
                    result->kind   = GV_NUMERIC;
                    result->string = strdup(ns);
                    pfree(ns);
                    break;
                }

                case jbvString:
                {
                    int len = jv->val.string.len;
                    result->kind   = GV_STRING;
                    result->string = (char *) malloc((size_t)len + 1);
                    if (result->string)
                    {
                        memcpy(result->string, jv->val.string.val, (size_t)len);
                        result->string[len] = '\0';
                    }
                    break;
                }

                default:
                    result->kind = GV_NULL;
                    break;
            }
            break;

        /* ── Array ──────────────────────────────────────────────────── */
        case WJB_BEGIN_ARRAY:
        {
            result->kind = GV_ARRAY;
            for (;;)
            {
                JsonbValue         ejv;
                JsonbIteratorToken etok;
                GenericValue      *elem;
                size_t             newcap;

                etok = JsonbIteratorNext(it, &ejv, false);
                if (etok == WJB_END_ARRAY)
                    break;

                elem = gv_build(it, etok, &ejv);

                if (result->arr.len == result->arr.cap)
                {
                    newcap = result->arr.cap ? result->arr.cap * 2 : 4;
                    result->arr.items = (GenericValue **)
                        realloc(result->arr.items,
                                newcap * sizeof(GenericValue *));
                    result->arr.cap = newcap;
                }
                result->arr.items[result->arr.len++] = elem;
            }
            break;
        }

        /* ── Object ─────────────────────────────────────────────────── */
        case WJB_BEGIN_OBJECT:
        {
            result->kind = GV_MAP;
            for (;;)
            {
                JsonbValue         kjv;
                JsonbValue         vvjv;
                JsonbIteratorToken ktok;
                JsonbIteratorToken vtok;
                char              *key;
                GenericValue      *val;
                size_t             newcap;
                int                klen;

                ktok = JsonbIteratorNext(it, &kjv, false);
                if (ktok == WJB_END_OBJECT)
                    break;
                /* ktok == WJB_KEY */
                klen = kjv.val.string.len;
                key  = (char *) malloc((size_t)klen + 1);
                if (key)
                {
                    memcpy(key, kjv.val.string.val, (size_t)klen);
                    key[klen] = '\0';
                }

                vtok = JsonbIteratorNext(it, &vvjv, false);
                val  = gv_build(it, vtok, &vvjv);

                if (result->map.len == result->map.cap)
                {
                    newcap = result->map.cap ? result->map.cap * 2 : 4;
                    result->map.ents = (MapEntry *)
                        realloc(result->map.ents, newcap * sizeof(MapEntry));
                    result->map.cap = newcap;
                }
                result->map.ents[result->map.len].key = key;
                result->map.ents[result->map.len].val = val;
                result->map.len++;
            }
            break;
        }

        default:
            result->kind = GV_NULL;
            break;
    }

    return result;
}

/* ============================================================================
 * GenericValue → JSON text
 *
 * Produces a valid JSON string into a StringInfo (palloc-backed).
 * ============================================================================ */

/* Forward declaration for recursive use */
static void gv_append_json(const GenericValue *gv, StringInfo si);

static void
append_json_string(StringInfo si, const char *str)
{
    const char    *p;
    unsigned char  c;

    appendStringInfoChar(si, '"');
    for (p = str; *p; p++)
    {
        c = (unsigned char) *p;
        if      (c == '"')  appendStringInfoString(si, "\\\"");
        else if (c == '\\') appendStringInfoString(si, "\\\\");
        else if (c == '\b') appendStringInfoString(si, "\\b");
        else if (c == '\f') appendStringInfoString(si, "\\f");
        else if (c == '\n') appendStringInfoString(si, "\\n");
        else if (c == '\r') appendStringInfoString(si, "\\r");
        else if (c == '\t') appendStringInfoString(si, "\\t");
        else if (c < 0x20)  appendStringInfo(si, "\\u%04x", (unsigned int)c);
        else                appendStringInfoChar(si, (char)c);
    }
    appendStringInfoChar(si, '"');
}

static void
gv_append_json(const GenericValue *gv, StringInfo si)
{
    size_t i;

    if (!gv) { appendStringInfoString(si, "null"); return; }

    switch (gv->kind)
    {
        case GV_NULL:
            appendStringInfoString(si, "null");
            break;
        case GV_BOOL:
            appendStringInfoString(si, gv->boolean ? "true" : "false");
            break;
        case GV_NUMERIC:
            /* numeric string written verbatim - no quotes */
            appendStringInfoString(si, gv->string ? gv->string : "0");
            break;
        case GV_STRING:
            append_json_string(si, gv->string ? gv->string : "");
            break;
        case GV_ARRAY:
            appendStringInfoChar(si, '[');
            for (i = 0; i < gv->arr.len; i++)
            {
                if (i > 0)
                    appendStringInfoChar(si, ',');
                gv_append_json(gv->arr.items[i], si);
            }
            appendStringInfoChar(si, ']');
            break;
        case GV_MAP:
            appendStringInfoChar(si, '{');
            for (i = 0; i < gv->map.len; i++)
            {
                if (i > 0)
                    appendStringInfoChar(si, ',');
                append_json_string(si,
                    gv->map.ents[i].key ? gv->map.ents[i].key : "");
                appendStringInfoChar(si, ':');
                gv_append_json(gv->map.ents[i].val, si);
            }
            appendStringInfoChar(si, '}');
            break;
    }
}

/* ============================================================================
 * yt_diff_apply(doc jsonb, diff bytea) RETURNS jsonb
 * ============================================================================ */

PG_FUNCTION_INFO_V1(yt_diff_apply);

Datum
yt_diff_apply(PG_FUNCTION_ARGS)
{
    Jsonb               *input_jb;
    bytea               *diff_bytes;
    const uint8_t       *pb_data;
    size_t               pb_len;
    DiffDoc__Mismatches *diff;
    GenericValue        *doc;
    DocError             errs[4];
    size_t               nerrs;
    int                  rc;
    StringInfoData       si;
    Datum                result;
    size_t               i;

    input_jb   = PG_GETARG_JSONB_P(0);
    diff_bytes = PG_GETARG_BYTEA_PP(1);
    pb_data    = (const uint8_t *) VARDATA_ANY(diff_bytes);
    pb_len     = VARSIZE_ANY_EXHDR(diff_bytes);
    nerrs      = 0;

    /* 1. Decode the protobuf-encoded Mismatches */
    diff = diff_doc__mismatches__unpack(NULL, pb_len, pb_data);
    if (diff == NULL)
        ereport(ERROR,
                (errcode(ERRCODE_INVALID_PARAMETER_VALUE),
                 errmsg("yt_diff_apply: failed to decode Mismatches protobuf")));

    /* 2. Convert jsonb → GenericValue tree */
    doc = gv_from_jsonb(input_jb);

    /* 3. Apply the diff in-place (fail_fast = 1: stop on first error) */
    rc = mismatches_apply_mut(diff, doc, 1, errs, lengthof(errs), &nerrs);

    diff_doc__mismatches__free_unpacked(diff, NULL);

    if (rc < 0 && nerrs > 0)
    {
        char *msg = pstrdup(errs[0].msg);

        for (i = 0; i < nerrs; i++)
            doc_error_free(&errs[i]);
        gv_free(doc);
        ereport(ERROR,
                (errcode(ERRCODE_DATA_EXCEPTION),
                 errmsg("yt_diff_apply: %s", msg)));
    }

    /* 4. Serialize the mutated GenericValue back to JSON text */
    initStringInfo(&si);
    gv_append_json(doc, &si);
    gv_free(doc);

    /* 5. Parse the JSON text into a jsonb datum and return it */
    result = DirectFunctionCall1(jsonb_in, CStringGetDatum(si.data));
    pfree(si.data);

    PG_RETURN_DATUM(result);
}

/* ============================================================================
 * BSON: a DocumentDB / MongoDB document as bytea
 *
 * The mapping of diff-doc-rs: {"$oid": ...},
 * {"$date": ...}, {"$numberLong": ...}, {"$numberDecimal": ...} ... - so a patch
 * the Rust side makes of two BSON documents applies here to the same paths.
 * Written back with its keys sorted, the _id first: each node the same bytes.
 * ============================================================================ */

/* the malloc'ed BSON as a palloc'd bytea; frees it */
static bytea *
bson_bytea(uint8_t *bytes, size_t len)
{
    bytea *out = (bytea *) palloc(VARHDRSZ + len);

    SET_VARSIZE(out, VARHDRSZ + len);
    memcpy(VARDATA(out), bytes, len);
    free(bytes);
    return out;
}

/* raise `err` as `fn`'s error, freeing it */
static void
bson_error(const char *fn, DocError *err)
{
    char *msg = pstrdup(err->msg ? err->msg : "BSON: out of memory");

    doc_error_free(err);
    ereport(ERROR,
            (errcode(ERRCODE_DATA_EXCEPTION),
             errmsg("%s: %s", fn, msg)));
}

static GenericValue *
gv_from_bytea(const char *fn, bytea *b)
{
    DocError      err;
    GenericValue *gv = gv_from_bson((const uint8_t *) VARDATA_ANY(b),
                                    VARSIZE_ANY_EXHDR(b), &err);

    if (!gv)
        bson_error(fn, &err);
    return gv;
}

/* writes `gv` and frees it */
static bytea *
gv_to_bytea(const char *fn, GenericValue *gv)
{
    DocError err;
    uint8_t *bytes;
    size_t   len;
    int      rc = gv_to_bson(gv, &bytes, &len, &err);

    gv_free(gv);
    if (rc != 0)
        bson_error(fn, &err);
    return bson_bytea(bytes, len);
}

PG_FUNCTION_INFO_V1(yt_diff_apply_bson);

/* yt_diff_apply_bson(doc bytea, diff bytea) RETURNS bytea */
Datum
yt_diff_apply_bson(PG_FUNCTION_ARGS)
{
    bytea               *doc_bytes  = PG_GETARG_BYTEA_PP(0);
    bytea               *diff_bytes = PG_GETARG_BYTEA_PP(1);
    DiffDoc__Mismatches *diff;
    GenericValue        *doc;
    DocError             errs[4];
    size_t               nerrs = 0;
    size_t               i;
    int                  rc;

    diff = diff_doc__mismatches__unpack(NULL, VARSIZE_ANY_EXHDR(diff_bytes),
                                        (const uint8_t *) VARDATA_ANY(diff_bytes));
    if (diff == NULL)
        ereport(ERROR,
                (errcode(ERRCODE_INVALID_PARAMETER_VALUE),
                 errmsg("yt_diff_apply_bson: failed to decode Mismatches protobuf")));

    {
        DocError err;

        doc = gv_from_bson((const uint8_t *) VARDATA_ANY(doc_bytes),
                           VARSIZE_ANY_EXHDR(doc_bytes), &err);
        if (!doc)
        {
            diff_doc__mismatches__free_unpacked(diff, NULL);
            bson_error("yt_diff_apply_bson", &err);
        }
    }

    rc = mismatches_apply_mut(diff, doc, 1, errs, lengthof(errs), &nerrs);
    diff_doc__mismatches__free_unpacked(diff, NULL);

    if (rc < 0 && nerrs > 0)
    {
        char *msg = pstrdup(errs[0].msg);

        for (i = 0; i < nerrs; i++)
            doc_error_free(&errs[i]);
        gv_free(doc);
        ereport(ERROR,
                (errcode(ERRCODE_DATA_EXCEPTION),
                 errmsg("yt_diff_apply_bson: %s", msg)));
    }

    PG_RETURN_BYTEA_P(gv_to_bytea("yt_diff_apply_bson", doc));
}

PG_FUNCTION_INFO_V1(yt_bson_to_json);

/* yt_bson_to_json(doc bytea) RETURNS jsonb: as extended JSON, relaxed */
Datum
yt_bson_to_json(PG_FUNCTION_ARGS)
{
    GenericValue  *doc = gv_from_bytea("yt_bson_to_json", PG_GETARG_BYTEA_PP(0));
    StringInfoData si;
    Datum          result;

    initStringInfo(&si);
    gv_append_json(doc, &si);
    gv_free(doc);
    result = DirectFunctionCall1(jsonb_in, CStringGetDatum(si.data));
    pfree(si.data);
    PG_RETURN_DATUM(result);
}

PG_FUNCTION_INFO_V1(yt_bson_from_json);

/* yt_bson_from_json(doc jsonb) RETURNS bytea: extended JSON's types read back */
Datum
yt_bson_from_json(PG_FUNCTION_ARGS)
{
    PG_RETURN_BYTEA_P(gv_to_bytea("yt_bson_from_json", gv_from_jsonb(PG_GETARG_JSONB_P(0))));
}
