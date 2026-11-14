/* SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent */
/*
 * protobuf.c - Protocol Buffer Encoding/Decoding
 *
 * Uses generated protobuf-c types from src/proto/ (base.pb-c.h / db.pb-c.h)
 * to encode custom YT Cluster types for gRPC calls and decode responses back.
 */

#include "yt_cluster.h"
#include "proto/db.pb-c.h"

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

/* ============================================================================
 * Internal helpers: initialise generated structs from custom types.
 *
 * These borrow pointers (strings, nested structs) from the source - they do
 * NOT deep-copy, so the generated structs must be used before the source is
 * freed and must NOT be freed via protobuf-c free functions.
 * ============================================================================ */

static void
init_peer_uuid(Base__PeerUuid *dst, const PeerUuid *src)
{
    base__peer_uuid__init(dst);
    dst->hi = src->hi;
    dst->lo = src->lo;
}

static void
init_entity_key(Base__EntityKey *dst, const EntityKey *src)
{
    base__entity_key__init(dst);
    dst->term    = src->term;
    dst->subterm = src->subterm;
    dst->version = src->version;
}

static void
init_var_type(Base__VarType *dst, const VarType *src)
{
    base__var_type__init(dst);
    switch (src->kind)
    {
        case VAR_TYPE_NULL:
            dst->value_case = BASE__VAR_TYPE__VALUE__NOT_SET;
            break;
        case VAR_TYPE_INT:
            dst->value_case = BASE__VAR_TYPE__VALUE_INT;
            dst->int_        = src->value.int_val;
            break;
        case VAR_TYPE_UINT:
            dst->value_case = BASE__VAR_TYPE__VALUE_UINT;
            dst->uint        = src->value.uint_val;
            break;
        case VAR_TYPE_FLOAT:
            dst->value_case = BASE__VAR_TYPE__VALUE_FLT;
            dst->flt         = src->value.float_val;
            break;
        case VAR_TYPE_BOOL:
            dst->value_case = BASE__VAR_TYPE__VALUE_BOOL;
            dst->bool_       = src->value.bool_val;
            break;
        case VAR_TYPE_UNIVERSAL:
            dst->value_case = BASE__VAR_TYPE__VALUE_UNIVERSAL;
            dst->universal   = src->value.str_val;
            break;
        case VAR_TYPE_DIFF:
            /* Diff encoding is not implemented; it needs DiffDoc__Mismatches construction */
            dst->value_case = BASE__VAR_TYPE__VALUE__NOT_SET;
            break;
        case VAR_TYPE_PATCH:
            /* Patch encoding is not implemented */
            dst->value_case  = BASE__VAR_TYPE__VALUE__NOT_SET;
            break;
    }
}

/* ============================================================================
 * DbInputEntity encoding (db.proto: DbInputEntity)
 * ============================================================================ */

size_t
yt_cluster_pb_encode_db_input_entity(const DbInputEntity *req, uint8_t **out)
{
    Db__DbInputEntity   msg     = DB__DB_INPUT_ENTITY__INIT;
    Base__TableRef      table   = BASE__TABLE_REF__INIT;
    Base__PeerUuid      origin  = BASE__PEER_UUID__INIT;

    Base__ColumnRef                         *cols       = NULL;
    Base__ColumnRef                        **col_ptrs   = NULL;
    Base__VarType                           *old_vars   = NULL;
    Db__DbInputEntity__OldDataEntry         *old_ents   = NULL;
    Db__DbInputEntity__OldDataEntry        **old_ptrs   = NULL;
    Base__VarType                           *new_vars   = NULL;
    Db__DbInputEntity__NewDataEntry         *new_ents   = NULL;
    Db__DbInputEntity__NewDataEntry        **new_ptrs   = NULL;

    size_t packed_size = 0;
    int    i;

    *out = NULL;

    msg.tx        = req->tx;
    msg.action    = (Db__DbAction) req->action;
    msg.iso       = (Db__PgIso)   req->iso;
    msg.user_role = req->user_role ? req->user_role : "";
    msg.jwt       = req->jwt       ? req->jwt       : "";

    /* table */
    if (req->table != NULL)
    {
        table.oid    = req->table->oid;
        table.db     = req->table->db     ? req->table->db     : "";
        table.schema = req->table->schema ? req->table->schema : "";
        table.table  = req->table->table  ? req->table->table  : "";

        if (req->table->columns_count > 0)
        {
            cols     = calloc(req->table->columns_count, sizeof(Base__ColumnRef));
            col_ptrs = calloc(req->table->columns_count, sizeof(Base__ColumnRef *));
            if (!cols || !col_ptrs)
                goto cleanup;

            for (i = 0; i < req->table->columns_count; i++)
            {
                const YtColumnRef *c = &req->table->columns[i];
                base__column_ref__init(&cols[i]);
                cols[i].attnum    = c->attnum;
                cols[i].oid       = c->oid;
                cols[i].name      = c->name ? c->name : "";
                cols[i].pk        = c->pk;
                cols[i].atttypid  = c->atttypid;
                cols[i].atttypmod = c->atttypmod;
                cols[i].ignore    = c->ignore;
                col_ptrs[i]       = &cols[i];
            }
            table.n_columns = (size_t) req->table->columns_count;
            table.columns   = col_ptrs;
        }
        msg.table = &table;
    }

    /* origin */
    if (req->origin != NULL)
    {
        init_peer_uuid(&origin, req->origin);
        msg.origin = &origin;
    }

    /* old_data */
    if (req->old_data_count > 0)
    {
        old_vars = calloc(req->old_data_count, sizeof(Base__VarType));
        old_ents = calloc(req->old_data_count, sizeof(Db__DbInputEntity__OldDataEntry));
        old_ptrs = calloc(req->old_data_count, sizeof(Db__DbInputEntity__OldDataEntry *));
        if (!old_vars || !old_ents || !old_ptrs)
            goto cleanup;

        for (i = 0; i < req->old_data_count; i++)
        {
            db__db_input_entity__old_data_entry__init(&old_ents[i]);
            old_ents[i].key = req->old_data[i].key;
            if (req->old_data[i].value != NULL)
            {
                init_var_type(&old_vars[i], req->old_data[i].value);
                old_ents[i].value = &old_vars[i];
            }
            old_ptrs[i] = &old_ents[i];
        }
        msg.n_old_data = (size_t) req->old_data_count;
        msg.old_data   = old_ptrs;
    }

    /* new_data */
    if (req->new_data_count > 0)
    {
        new_vars = calloc(req->new_data_count, sizeof(Base__VarType));
        new_ents = calloc(req->new_data_count, sizeof(Db__DbInputEntity__NewDataEntry));
        new_ptrs = calloc(req->new_data_count, sizeof(Db__DbInputEntity__NewDataEntry *));
        if (!new_vars || !new_ents || !new_ptrs)
            goto cleanup;

        for (i = 0; i < req->new_data_count; i++)
        {
            db__db_input_entity__new_data_entry__init(&new_ents[i]);
            new_ents[i].key = req->new_data[i].key;
            if (req->new_data[i].value != NULL)
            {
                init_var_type(&new_vars[i], req->new_data[i].value);
                new_ents[i].value = &new_vars[i];
            }
            new_ptrs[i] = &new_ents[i];
        }
        msg.n_new_data = (size_t) req->new_data_count;
        msg.new_data   = new_ptrs;
    }

    packed_size = db__db_input_entity__get_packed_size(&msg);
    *out = malloc(packed_size > 0 ? packed_size : 1);
    if (*out != NULL)
        db__db_input_entity__pack(&msg, *out);
    else
        packed_size = 0;

cleanup:
    free(cols);
    free(col_ptrs);
    free(old_vars);
    free(old_ents);
    free(old_ptrs);
    free(new_vars);
    free(new_ents);
    free(new_ptrs);

    return packed_size;
}

/* ============================================================================
 * ClusterComplete encoding (base.proto: ClusterComplete)
 * ============================================================================ */

size_t
yt_cluster_pb_encode_cluster_complete(const ClusterComplete *req, uint8_t **out)
{
    Base__ClusterComplete msg  = BASE__CLUSTER_COMPLETE__INIT;
    Base__PeerUuid        orig = BASE__PEER_UUID__INIT;
    Base__EntityKey       key  = BASE__ENTITY_KEY__INIT;
    Base__EntityKey       prev = BASE__ENTITY_KEY__INIT;
    size_t packed_size;

    *out = NULL;

    msg.txid   = req->txid;
    msg.commit = (Base__ClusterCommitType) req->commit;

    if (req->orig != NULL)
    {
        init_peer_uuid(&orig, req->orig);
        msg.orig = &orig;
    }
    if (req->key != NULL)
    {
        init_entity_key(&key, req->key);
        msg.key = &key;
    }
    if (req->prev != NULL)
    {
        init_entity_key(&prev, req->prev);
        msg.prev = &prev;
    }

    packed_size = base__cluster_complete__get_packed_size(&msg);
    *out = malloc(packed_size > 0 ? packed_size : 1);
    if (*out != NULL)
        base__cluster_complete__pack(&msg, *out);
    else
        packed_size = 0;

    return packed_size;
}

/* ============================================================================
 * InfoRequest encoding (db.proto: InfoRequest)
 * ============================================================================ */

size_t
yt_cluster_pb_encode_info_request(const InfoRequest *req, uint8_t **out)
{
    Db__InfoRequest msg = DB__INFO_REQUEST__INIT;
    size_t packed_size;

    *out = NULL;

    msg.input = req->input ? req->input : "";

    packed_size = db__info_request__get_packed_size(&msg);
    *out = malloc(packed_size > 0 ? packed_size : 1);
    if (*out != NULL)
        db__info_request__pack(&msg, *out);
    else
        packed_size = 0;

    return packed_size;
}

/* ============================================================================
 * LlmRequest encoding (db.proto: LlmRequest)
 * ============================================================================ */

size_t
yt_cluster_pb_encode_llm_request(const LlmRequest *req, uint8_t **out)
{
    Db__LlmRequest msg = DB__LLM_REQUEST__INIT;
    size_t packed_size;

    *out = NULL;

    msg.question = req->question ? req->question : "";
    msg.db = req->db ? req->db : "";
    msg.role = req->role ? req->role : "";
    msg.n_schemas = (size_t) req->schemas_count;
    msg.schemas = req->schemas;
    msg.page = req->page;
    msg.size = req->size;

    packed_size = db__llm_request__get_packed_size(&msg);
    *out = malloc(packed_size > 0 ? packed_size : 1);
    if (*out != NULL)
        db__llm_request__pack(&msg, *out);
    else
        packed_size = 0;

    return packed_size;
}

/* ============================================================================
 * MergeRequest encoding (db.proto: MergeRequest)
 * ============================================================================ */

size_t
yt_cluster_pb_encode_merge_request(const MergeRequest *req, uint8_t **out)
{
    Db__MergeRequest msg = DB__MERGE_REQUEST__INIT;
    Base__EntityKey  key = BASE__ENTITY_KEY__INIT;
    size_t packed_size;

    *out = NULL;

    if (req->key != NULL)
    {
        init_entity_key(&key, req->key);
        msg.key = &key;
    }
    msg.pull         = req->pull;
    msg.force_commit = req->force_commit;

    packed_size = db__merge_request__get_packed_size(&msg);
    *out = malloc(packed_size > 0 ? packed_size : 1);
    if (*out != NULL)
        db__merge_request__pack(&msg, *out);
    else
        packed_size = 0;

    return packed_size;
}

/* ============================================================================
 * ModifyReply decoding (db.proto: ModifyReply)
 * ============================================================================ */

int
yt_cluster_pb_decode_modify_reply(const uint8_t *data, size_t len, ModifyReply *resp)
{
    Db__ModifyReply *msg = db__modify_reply__unpack(NULL, len, data);

    if (msg == NULL)
        return -1;

    memset(resp, 0, sizeof(*resp));
    resp->result = msg->result;
    resp->skip_complete = msg->skip_complete;

    if (msg->dump != NULL)
    {
        resp->dump = malloc(sizeof(DataVersionDumpTo));
        if (resp->dump != NULL)
        {
            memset(resp->dump, 0, sizeof(DataVersionDumpTo));
            resp->dump->table     = msg->dump->table ? strdup(msg->dump->table) : NULL;
            resp->dump->term_base = msg->dump->term_base;

            if (msg->dump->data_version_dump != NULL)
            {
                resp->dump->data_version_dump = malloc(sizeof(DataVersionDump));
                if (resp->dump->data_version_dump != NULL)
                {
                    resp->dump->data_version_dump->term_delta =
                        msg->dump->data_version_dump->term_delta;
                    resp->dump->data_version_dump->subterm =
                        msg->dump->data_version_dump->subterm;
                }
            }
        }
    }

    db__modify_reply__free_unpacked(msg, NULL);
    return 0;
}

/* ============================================================================
 * CompleteReply decoding (db.proto: CompleteReply)
 * ============================================================================ */

int
yt_cluster_pb_decode_complete_reply(const uint8_t *data, size_t len, CompleteReply *resp)
{
    Db__CompleteReply *msg = db__complete_reply__unpack(NULL, len, data);

    if (msg == NULL)
        return -1;

    memset(resp, 0, sizeof(*resp));
    resp->result = msg->result;

    db__complete_reply__free_unpacked(msg, NULL);
    return 0;
}

/* ============================================================================
 * InfoResponse decoding (db.proto: InfoResponse)
 * ============================================================================ */

int
yt_cluster_pb_decode_info_response(const uint8_t *data, size_t len, InfoResponse *resp)
{
    Db__InfoResponse *msg = db__info_response__unpack(NULL, len, data);
    size_t i;

    if (msg == NULL)
        return -1;

    memset(resp, 0, sizeof(*resp));

    if (msg->n_data > 0)
    {
        resp->data = malloc(sizeof(CfgVo) * msg->n_data);
        if (resp->data == NULL)
        {
            db__info_response__free_unpacked(msg, NULL);
            return -1;
        }
        memset(resp->data, 0, sizeof(CfgVo) * msg->n_data);

        for (i = 0; i < msg->n_data; i++)
        {
            Db__CfgVo *src    = msg->data[i];
            resp->data[i].name   = src->name   ? strdup(src->name)   : NULL;
            resp->data[i].value  = src->value  ? strdup(src->value)  : NULL;
            resp->data[i].module = src->module ? strdup(src->module) : NULL;
        }
        resp->data_count = (int) msg->n_data;
    }

    db__info_response__free_unpacked(msg, NULL);
    return 0;
}

/* ============================================================================
 * Free functions
 * ============================================================================ */

void
yt_cluster_pb_free_modify_reply(ModifyReply *resp)
{
    if (resp->dump != NULL)
    {
        if (resp->dump->table != NULL)
            free(resp->dump->table);
        if (resp->dump->data_version_dump != NULL)
            free(resp->dump->data_version_dump);
        free(resp->dump);
    }
    memset(resp, 0, sizeof(*resp));
}

/* LlmReply decoding; the strings are malloc'ed, see yt_cluster_pb_free_llm_reply() */
int
yt_cluster_pb_decode_llm_reply(const uint8_t *data, size_t len, LlmReply *resp)
{
    Db__LlmReply *msg = db__llm_reply__unpack(NULL, len, data);

    if (msg == NULL)
        return -1;

    memset(resp, 0, sizeof(*resp));
    resp->type = msg->type ? strdup(msg->type) : NULL;
    resp->sql  = msg->sql  ? strdup(msg->sql)  : NULL;
    resp->json = msg->json ? strdup(msg->json) : NULL;

    db__llm_reply__free_unpacked(msg, NULL);
    return 0;
}

void
yt_cluster_pb_free_llm_reply(LlmReply *resp)
{
    free(resp->type);
    free(resp->sql);
    free(resp->json);
    memset(resp, 0, sizeof(*resp));
}

void
yt_cluster_pb_free_info_response(InfoResponse *resp)
{
    int i;

    if (resp->data != NULL)
    {
        for (i = 0; i < resp->data_count; i++)
        {
            if (resp->data[i].name   != NULL) free(resp->data[i].name);
            if (resp->data[i].value  != NULL) free(resp->data[i].value);
            if (resp->data[i].module != NULL) free(resp->data[i].module);
        }
        free(resp->data);
    }
    memset(resp, 0, sizeof(*resp));
}

void
yt_cluster_pb_free_db_input_entity(DbInputEntity *req)
{
    int i;

    if (req->table != NULL)
    {
        if (req->table->db     != NULL) pfree(req->table->db);
        if (req->table->schema != NULL) pfree(req->table->schema);
        if (req->table->table  != NULL) pfree(req->table->table);
        if (req->table->columns != NULL)
        {
            for (i = 0; i < req->table->columns_count; i++)
            {
                if (req->table->columns[i].name != NULL)
                    pfree(req->table->columns[i].name);
            }
            pfree(req->table->columns);
        }
        pfree(req->table);
    }

    if (req->old_data != NULL)
    {
        for (i = 0; i < req->old_data_count; i++)
        {
            if (req->old_data[i].value != NULL)
                pfree(req->old_data[i].value);
        }
        pfree(req->old_data);
    }

    if (req->new_data != NULL)
    {
        for (i = 0; i < req->new_data_count; i++)
        {
            if (req->new_data[i].value != NULL)
                pfree(req->new_data[i].value);
        }
        pfree(req->new_data);
    }

    memset(req, 0, sizeof(*req));
}
