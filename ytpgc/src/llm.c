/* SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent */
/*
 * llm.c - yt_llm_query(): a plain-language question in, SQL or rows out
 *
 * The text-to-SQL model is a service on the LAN, behind a proxy that takes only
 * clients presenting a certificate from a private CA. The node
 * does all of the model call: it describes this database to
 * the model, opens the client keys kept encrypted in module 'K' with its app
 * key, asks, and answers with the statement - or, for a page, with its rows,
 * read as the caller. yt_llm() in yt_cluster--1.21.sql turns either into rows.
 *
 * The node may also answer from ClickHouse ("ch") or from
 * DataFusion over the cluster's tables ("df"): it chooses by the tables the
 * model's statement reads. Those statements run where the node chose, never
 * here, so yt_llm() asks the node for their rows.
 */

#include "yt_cluster.h"

#include "catalog/namespace.h"
#include "commands/dbcommands.h"
#include "miscadmin.h"
#include "nodes/parsenodes.h"
#include "parser/parser.h"
#include "utils/lsyscache.h"

PG_FUNCTION_INFO_V1(yt_llm_query);

static void
llm_sql_context(void *arg)
{
    errcontext("the model's statement: %s", (const char *) arg);
}

/*
 * yt_llm() runs an answer of type "sql" as it is (RETURN QUERY EXECUTE), so it
 * is parsed here first: exactly one SELECT, no row locks, no WITH that writes.
 * yt_llm() also runs read-only; this makes the refusal a clear one, and before
 * anything runs. A "df" statement is PostgreSQL's dialect too (DataFusion
 * parses it as such), so it gets the same check.
 */
static void
llm_check_select(const char *sql)
{
    ErrorContextCallback errcallback;
    List       *raw;
    SelectStmt *stmt;
    ListCell   *lc;

    errcallback.callback = llm_sql_context;
    errcallback.arg = (void *) sql;
    errcallback.previous = error_context_stack;
    error_context_stack = &errcallback;

    raw = raw_parser(sql, RAW_PARSE_DEFAULT);
    if (list_length(raw) != 1 || !IsA(linitial_node(RawStmt, raw)->stmt, SelectStmt))
        ereport(ERROR,
                (errcode(ERRCODE_FEATURE_NOT_SUPPORTED),
                 errmsg("yt_llm_query: the model's statement is not one SELECT")));
    stmt = (SelectStmt *) linitial_node(RawStmt, raw)->stmt;
    if (stmt->lockingClause != NIL)
        ereport(ERROR,
                (errcode(ERRCODE_FEATURE_NOT_SUPPORTED),
                 errmsg("yt_llm_query: the model's statement locks rows")));
    if (stmt->withClause != NULL)
    {
        foreach(lc, stmt->withClause->ctes)
        {
            CommonTableExpr *cte = lfirst_node(CommonTableExpr, lc);

            if (!IsA(cte->ctequery, SelectStmt))
                ereport(ERROR,
                        (errcode(ERRCODE_FEATURE_NOT_SUPPORTED),
                         errmsg("yt_llm_query: the model's statement writes, in WITH %s", cte->ctename)));
        }
    }

    error_context_stack = errcallback.previous;
}

/*
 * yt_llm_query(nl text, page int DEFAULT NULL, size int DEFAULT NULL) RETURNS json
 *
 * Calls the node with the question and where it was asked: this database, the
 * current role (the node describes only the tables it may read, and reads a
 * page as it) and the schemas of search_path. Answers:
 *   {"type":"sql","sql":"SELECT ..."}                       - page and size NULL
 *   {"type":"ch","sql":"SELECT ...","notes":[...]}          - ClickHouse SQL
 *   {"type":"df","sql":"SELECT ...","script":"...","plan":{...},"notes":[...]}
 *                                                           - DataFusion
 *   {"type":"data","engine":"sql|ch|df","data":[{...}],"column_names":[...],
 *    "column_desc":[...],"page":0,"page_size":10,"total_rows":9}
 *                                                           - a page: page 0 first,
 *                                                             size 10 unless given
 * No time limit: the model thinks for minutes on a CPU, and a page of an
 * analytic statement may run for many more. A cancel of the statement - or its
 * statement_timeout - ends the call, and the node then hangs up on the model
 * and cancels the page it runs (grpc_client.c cancel_check).
 */
Datum
yt_llm_query(PG_FUNCTION_ARGS)
{
    LlmRequest  req;
    LlmReply    resp;
    List       *path;
    ListCell   *lc;
    char       *type;
    char       *sql;
    char       *json;
    int         i = 0;
    int         ret;

    if (PG_ARGISNULL(0))
        PG_RETURN_NULL();

    memset(&req, 0, sizeof(req));
    req.question = text_to_cstring(PG_GETARG_TEXT_PP(0));
    req.db = get_database_name(MyDatabaseId);
    req.role = GetUserNameFromId(GetUserId(), false);

    /* current_schemas(false): where an unqualified name would be looked for */
    path = fetch_search_path(false);
    req.schemas = palloc0(sizeof(char *) * Max(list_length(path), 1));
    foreach(lc, path)
    {
        char   *nsp = get_namespace_name(lfirst_oid(lc));

        if (nsp != NULL)
            req.schemas[i++] = nsp;
    }
    req.schemas_count = i;
    list_free(path);

    if (!PG_ARGISNULL(1) || !PG_ARGISNULL(2))
    {
        int32   page = PG_ARGISNULL(1) ? 0 : PG_GETARG_INT32(1);
        int32   size = PG_ARGISNULL(2) ? 10 : PG_GETARG_INT32(2);

        if (page < 0 || size < 1)
            ereport(ERROR,
                    (errcode(ERRCODE_INVALID_PARAMETER_VALUE),
                     errmsg("yt_llm_query: page must be 0 or more, and size 1 or more")));
        req.page = (uint32_t) page;
        req.size = (uint32_t) size;
    }

    yt_cluster_ensure_connected("yt_llm_query");

    memset(&resp, 0, sizeof(resp));
    ret = yt_cluster_grpc_llm(&req, &resp);
    if (ret != 0)
    {
        const char *e = yt_cluster_grpc_last_error();

        CHECK_FOR_INTERRUPTS();     /* a cancel ended the call: raise the cancel */
        ereport(ERROR,
                (errcode(ERRCODE_CONNECTION_FAILURE),
                 errmsg("yt_llm_query: %s", e),
                 strstr(e, "status 12") != NULL
                 ? errhint("This node's ytserv has no llm call yet: it predates the release that added it.") : 0));
    }

    /* out of malloc'ed memory before anything can ereport */
    type = pstrdup(resp.type != NULL ? resp.type : "");
    sql = pstrdup(resp.sql != NULL ? resp.sql : "");
    json = pstrdup(resp.json != NULL ? resp.json : "");
    yt_cluster_pb_free_llm_reply(&resp);

    if (strcmp(type, "sql") == 0 || strcmp(type, "df") == 0)
        llm_check_select(sql);
    else if (strcmp(type, "ch") != 0 && strcmp(type, "data") != 0)     /* ClickHouse checked "ch" */
        ereport(ERROR,
                (errcode(ERRCODE_PROTOCOL_VIOLATION),
                 errmsg("yt_llm_query: the node answered \"%s\", none of sql, ch, df and data", type)));

    PG_RETURN_DATUM(DirectFunctionCall1(json_in, CStringGetDatum(json)));
}
