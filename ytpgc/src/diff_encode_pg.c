/* SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent */
/*
 * diff_encode_pg.c - yt_diff_encode() PostgreSQL function
 *
 * Signature: yt_diff_encode(diff jsonb) RETURNS bytea
 *
 * Parses a protobuf JSON-encoded DiffDoc::Mismatches (as jsonb) and
 * serialises it into protobuf binary (bytea).
 *
 * JSON format mirrors the proto3 field names, e.g.:
 *   {"doc": {"hunks": [
 *     {"path": [{"name": "field"}], "value": {"update": {"string": "new"}}}
 *   ]}}
 */

#include "yt_cluster.h"
#include "proto/diff_doc.pb-c.h"

#include "utils/jsonb.h"
#include "utils/builtins.h"
#include "utils/numeric.h"

#include <string.h>

/* ============================================================================
 * Low-level jsonb helpers  (all allocations via palloc)
 * ============================================================================ */

/*
 * Look up a field by name in a jsonb object container.
 * Returns NULL when the key is absent.
 */
static JsonbValue *
jb_field(JsonbContainer *c, const char *key)
{
    JsonbValue kv;

    kv.type            = jbvString;
    kv.val.string.val  = (char *) key;
    kv.val.string.len  = (int) strlen(key);
    return findJsonbValueFromContainer(c, JB_FOBJECT, &kv);
}

/*
 * Extract a NUL-terminated palloc'd string from a jbvString JsonbValue.
 * Returns NULL on type mismatch.
 */
static char *
jbv_str(const JsonbValue *jv)
{
    char *s;

    if (!jv || jv->type != jbvString)
        return NULL;
    s = palloc(jv->val.string.len + 1);
    memcpy(s, jv->val.string.val, jv->val.string.len);
    s[jv->val.string.len] = '\0';
    return s;
}

/*
 * Collect all elements of a jsonb array into a palloc'd array of JsonbValue*.
 * Nested objects/arrays are returned as jbvBinary entries (skipNested=true).
 * *n_out receives the element count.
 */
static JsonbValue **
jb_array_elems(JsonbContainer *c, int *n_out)
{
    JsonbIterator     *it;
    JsonbValue         jv;
    JsonbIteratorToken tok;
    JsonbValue       **elems = NULL;
    int                n     = 0;
    int                cap   = 0;

    it  = JsonbIteratorInit(c);
    tok = JsonbIteratorNext(&it, &jv, false); /* consume WJB_BEGIN_ARRAY */
    (void) tok;

    for (;;)
    {
        tok = JsonbIteratorNext(&it, &jv, true); /* skipNested → jbvBinary */
        if (tok == WJB_END_ARRAY)
            break;

        if (n == cap)
        {
            cap   = cap ? cap * 2 : 4;
            elems = elems
                ? (JsonbValue **) repalloc(elems, (Size)(cap * (int)sizeof(JsonbValue *)))
                : (JsonbValue **) palloc((Size)(cap * (int)sizeof(JsonbValue *)));
        }
        elems[n] = (JsonbValue *) palloc(sizeof(JsonbValue));
        *elems[n] = jv;
        n++;
    }

    *n_out = n;
    return elems;
}

/* ============================================================================
 * Forward declarations
 * ============================================================================ */

static DiffDoc__GenericValue *parse_gv(JsonbContainer *c);
static DiffDoc__GenericMap   *parse_gmap(JsonbContainer *c);
static DiffDoc__GenericArray *parse_garr(JsonbContainer *c);
static DiffDoc__DocIndex     *parse_doc_index(JsonbContainer *c);
static DiffDoc__HunkAction   *parse_hunk_action(JsonbContainer *c);
static DiffDoc__Hunk         *parse_hunk(JsonbContainer *c);
static DiffDoc__DocMismatch  *parse_doc_mismatch(JsonbContainer *c);
static DiffDoc__Mismatches   *parse_mismatches(JsonbContainer *c);

/* ============================================================================
 * Parse GenericValue  {"string":"…"} | {"numeric":"…"} | {"boolean":true}
 *                     | {"null":true} | {"array":{…}} | {"map":{…}}
 * ============================================================================ */

static DiffDoc__GenericValue *
parse_gv(JsonbContainer *c)
{
    DiffDoc__GenericValue *gv = palloc(sizeof(*gv));
    JsonbValue            *fv;

    diff_doc__generic_value__init(gv);

    if ((fv = jb_field(c, "string")) != NULL)
    {
        gv->kind_case = DIFF_DOC__GENERIC_VALUE__KIND_STRING;
        gv->string    = jbv_str(fv);
    }
    else if ((fv = jb_field(c, "numeric")) != NULL)
    {
        gv->kind_case = DIFF_DOC__GENERIC_VALUE__KIND_NUMERIC;
        gv->numeric   = jbv_str(fv);
    }
    else if ((fv = jb_field(c, "boolean")) != NULL)
    {
        gv->kind_case = DIFF_DOC__GENERIC_VALUE__KIND_BOOLEAN;
        gv->boolean   = (fv->type == jbvBool) ? (fv->val.boolean ? 1 : 0) : 0;
    }
    else if (jb_field(c, "null") != NULL)
    {
        gv->kind_case = DIFF_DOC__GENERIC_VALUE__KIND_NULL;
        gv->null      = 1;
    }
    else if ((fv = jb_field(c, "array")) != NULL)
    {
        gv->kind_case = DIFF_DOC__GENERIC_VALUE__KIND_ARRAY;
        if (fv->type == jbvBinary)
            gv->array = parse_garr(fv->val.binary.data);
    }
    else if ((fv = jb_field(c, "map")) != NULL)
    {
        gv->kind_case = DIFF_DOC__GENERIC_VALUE__KIND_MAP;
        if (fv->type == jbvBinary)
            gv->map = parse_gmap(fv->val.binary.data);
    }

    return gv;
}

/* ============================================================================
 * Parse GenericArray  {"items": [ <GenericValue>, … ]}
 * ============================================================================ */

static DiffDoc__GenericArray *
parse_garr(JsonbContainer *c)
{
    DiffDoc__GenericArray *ga  = palloc(sizeof(*ga));
    JsonbValue            *fv;

    diff_doc__generic_array__init(ga);

    fv = jb_field(c, "items");
    if (fv && fv->type == jbvBinary && JsonContainerIsArray(fv->val.binary.data))
    {
        JsonbValue **elems;
        int          n;
        int          i;

        elems      = jb_array_elems(fv->val.binary.data, &n);
        ga->n_items = (size_t) n;
        ga->items   = palloc((Size)(n * (int)sizeof(DiffDoc__GenericValue *)));

        for (i = 0; i < n; i++)
        {
            if (elems[i]->type == jbvBinary)
                ga->items[i] = parse_gv(elems[i]->val.binary.data);
            else
            {
                ga->items[i] = palloc(sizeof(DiffDoc__GenericValue));
                diff_doc__generic_value__init(ga->items[i]);
            }
        }
    }

    return ga;
}

/* ============================================================================
 * Parse GenericMap  {"fields": {"key": <GenericValue>, …}}
 * ============================================================================ */

static DiffDoc__GenericMap *
parse_gmap(JsonbContainer *c)
{
    DiffDoc__GenericMap *gm = palloc(sizeof(*gm));
    JsonbValue          *fv;

    diff_doc__generic_map__init(gm);

    fv = jb_field(c, "fields");
    if (fv && fv->type == jbvBinary && JsonContainerIsObject(fv->val.binary.data))
    {
        JsonbContainer    *fc = fv->val.binary.data;
        JsonbIterator     *it;
        JsonbValue         kv;
        JsonbValue         vv;
        JsonbIteratorToken tok;
        DiffDoc__GenericMap__FieldsEntry **fe_arr = NULL;
        int                               cnt     = 0;
        int                               cap     = 0;

        it  = JsonbIteratorInit(fc);
        tok = JsonbIteratorNext(&it, &kv, false); /* consume WJB_BEGIN_OBJECT */
        (void) tok;

        for (;;)
        {
            DiffDoc__GenericMap__FieldsEntry *fe;

            tok = JsonbIteratorNext(&it, &kv, false); /* WJB_KEY or WJB_END_OBJECT */
            if (tok == WJB_END_OBJECT)
                break;

            tok = JsonbIteratorNext(&it, &vv, true);  /* WJB_VALUE, skipNested */
            (void) tok;

            fe = palloc(sizeof(*fe));
            diff_doc__generic_map__fields_entry__init(fe);
            fe->key   = jbv_str(&kv);
            fe->value = (vv.type == jbvBinary)
                        ? parse_gv(vv.val.binary.data)
                        : NULL;

            if (cnt == cap)
            {
                cap    = cap ? cap * 2 : 4;
                fe_arr = fe_arr
                    ? (DiffDoc__GenericMap__FieldsEntry **)
                        repalloc(fe_arr, (Size)(cap * (int)sizeof(*fe_arr)))
                    : (DiffDoc__GenericMap__FieldsEntry **)
                        palloc((Size)(cap * (int)sizeof(*fe_arr)));
            }
            fe_arr[cnt++] = fe;
        }

        gm->n_fields = (size_t) cnt;
        gm->fields   = fe_arr;
    }

    return gm;
}

/* ============================================================================
 * Parse DocIndex  {"name": "field"} | {"idx": 0}
 * ============================================================================ */

static DiffDoc__DocIndex *
parse_doc_index(JsonbContainer *c)
{
    DiffDoc__DocIndex *di = palloc(sizeof(*di));
    JsonbValue        *fv;

    diff_doc__doc_index__init(di);

    if ((fv = jb_field(c, "name")) != NULL)
    {
        di->kind_case = DIFF_DOC__DOC_INDEX__KIND_NAME;
        di->name      = jbv_str(fv);
    }
    else if ((fv = jb_field(c, "idx")) != NULL && fv->type == jbvNumeric)
    {
        Datum d;

        di->kind_case = DIFF_DOC__DOC_INDEX__KIND_IDX;
        d             = DirectFunctionCall1(numeric_int8,
                                            NumericGetDatum(fv->val.numeric));
        di->idx       = (uint64_t) DatumGetInt64(d);
    }

    return di;
}

/* ============================================================================
 * Parse HunkAction  {"remove":true} | {"update":{…}} | {"insert":{…}}
 *                   | {"swap":{…}}  | {"clone":{…}}
 * ============================================================================ */

static DiffDoc__HunkAction *
parse_hunk_action(JsonbContainer *c)
{
    DiffDoc__HunkAction *ha = palloc(sizeof(*ha));
    JsonbValue          *fv;

    diff_doc__hunk_action__init(ha);

    if (jb_field(c, "remove") != NULL)
    {
        ha->kind_case = DIFF_DOC__HUNK_ACTION__KIND_REMOVE;
        ha->remove    = 1;
    }
    else if ((fv = jb_field(c, "update")) != NULL)
    {
        ha->kind_case = DIFF_DOC__HUNK_ACTION__KIND_UPDATE;
        if (fv->type == jbvBinary)
            ha->update = parse_gv(fv->val.binary.data);
    }
    else if ((fv = jb_field(c, "insert")) != NULL)
    {
        ha->kind_case = DIFF_DOC__HUNK_ACTION__KIND_INSERT;
        if (fv->type == jbvBinary)
            ha->insert = parse_gv(fv->val.binary.data);
    }
    else if ((fv = jb_field(c, "swap")) != NULL)
    {
        ha->kind_case = DIFF_DOC__HUNK_ACTION__KIND_SWAP;
        if (fv->type == jbvBinary)
            ha->swap = parse_doc_index(fv->val.binary.data);
    }
    else if ((fv = jb_field(c, "clone")) != NULL)
    {
        ha->kind_case = DIFF_DOC__HUNK_ACTION__KIND_CLONE;
        if (fv->type == jbvBinary)
            ha->clone = parse_doc_index(fv->val.binary.data);
    }

    return ha;
}

/* ============================================================================
 * Parse Hunk  {"path": […], "value": {…}}
 * ============================================================================ */

static DiffDoc__Hunk *
parse_hunk(JsonbContainer *c)
{
    DiffDoc__Hunk *h = palloc(sizeof(*h));
    JsonbValue    *fv;

    diff_doc__hunk__init(h);

    fv = jb_field(c, "path");
    if (fv && fv->type == jbvBinary && JsonContainerIsArray(fv->val.binary.data))
    {
        JsonbValue **elems;
        int          n;
        int          i;

        elems    = jb_array_elems(fv->val.binary.data, &n);
        h->n_path = (size_t) n;
        h->path   = palloc((Size)(n * (int)sizeof(DiffDoc__DocIndex *)));

        for (i = 0; i < n; i++)
        {
            if (elems[i]->type == jbvBinary)
                h->path[i] = parse_doc_index(elems[i]->val.binary.data);
            else
            {
                h->path[i] = palloc(sizeof(DiffDoc__DocIndex));
                diff_doc__doc_index__init(h->path[i]);
            }
        }
    }

    fv = jb_field(c, "value");
    if (fv && fv->type == jbvBinary)
        h->value = parse_hunk_action(fv->val.binary.data);

    return h;
}

/* ============================================================================
 * Parse DocMismatch  {"hunks": [ <Hunk>, … ]}
 * ============================================================================ */

static DiffDoc__DocMismatch *
parse_doc_mismatch(JsonbContainer *c)
{
    DiffDoc__DocMismatch *dm = palloc(sizeof(*dm));
    JsonbValue           *fv;

    diff_doc__doc_mismatch__init(dm);

    fv = jb_field(c, "hunks");
    if (fv && fv->type == jbvBinary && JsonContainerIsArray(fv->val.binary.data))
    {
        JsonbValue **elems;
        int          n;
        int          i;

        elems      = jb_array_elems(fv->val.binary.data, &n);
        dm->n_hunks = (size_t) n;
        dm->hunks   = palloc((Size)(n * (int)sizeof(DiffDoc__Hunk *)));

        for (i = 0; i < n; i++)
        {
            if (elems[i]->type == jbvBinary)
                dm->hunks[i] = parse_hunk(elems[i]->val.binary.data);
            else
            {
                dm->hunks[i] = palloc(sizeof(DiffDoc__Hunk));
                diff_doc__hunk__init(dm->hunks[i]);
            }
        }
    }

    return dm;
}

/* ============================================================================
 * Parse Mismatches  {"doc":{…}} | {"patch":"…"} | {"text":{…}}
 * ============================================================================ */

static DiffDoc__Mismatches *
parse_mismatches(JsonbContainer *c)
{
    DiffDoc__Mismatches *ms = palloc(sizeof(*ms));
    JsonbValue          *fv;

    diff_doc__mismatches__init(ms);

    if ((fv = jb_field(c, "doc")) != NULL)
    {
        ms->kind_case = DIFF_DOC__MISMATCHES__KIND_DOC;
        if (fv->type == jbvBinary)
            ms->doc = parse_doc_mismatch(fv->val.binary.data);
    }
    else if ((fv = jb_field(c, "patch")) != NULL)
    {
        ms->kind_case = DIFF_DOC__MISMATCHES__KIND_PATCH;
        ms->patch     = jbv_str(fv);
    }
    else if (jb_field(c, "text") != NULL)
    {
        /* TxtMismatch encoding not yet implemented */
        ereport(ERROR,
                (errcode(ERRCODE_FEATURE_NOT_SUPPORTED),
                 errmsg("yt_diff_encode: text (TxtMismatch) variant not yet supported")));
    }

    return ms;
}

/* ============================================================================
 * yt_diff_encode(diff jsonb) RETURNS bytea
 * ============================================================================ */

PG_FUNCTION_INFO_V1(yt_diff_encode);

Datum
yt_diff_encode(PG_FUNCTION_ARGS)
{
    Jsonb               *jb;
    DiffDoc__Mismatches *ms;
    size_t               packed_size;
    bytea               *result;
    uint8_t             *buf;

    jb = PG_GETARG_JSONB_P(0);

    if (!JsonContainerIsObject(&jb->root))
        ereport(ERROR,
                (errcode(ERRCODE_INVALID_PARAMETER_VALUE),
                 errmsg("yt_diff_encode: argument must be a JSON object")));

    ms          = parse_mismatches(&jb->root);
    packed_size = diff_doc__mismatches__get_packed_size(ms);
    result      = (bytea *) palloc(VARHDRSZ + packed_size);
    SET_VARSIZE(result, VARHDRSZ + packed_size);
    buf = (uint8_t *) VARDATA(result);
    diff_doc__mismatches__pack(ms, buf);

    PG_RETURN_BYTEA_P(result);
}
