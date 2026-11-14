#!/usr/bin/env python3
# SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
"""The extension's update scripts, made from its script: yt_cluster--<earlier>--<version>.sql for
every earlier version in previous-versions.txt, <version> the major.minor of ../VERSION.

    python3 update-sql.py            write them (make update-sql)
    python3 update-sql.py --check    exit 1 when one on disk is not what would be written

`ALTER EXTENSION yt_cluster UPDATE` runs one in a database that has an earlier version. It brings
that database to what `CREATE EXTENSION` makes now, an object at a time, each in a block of its
own: a function or a view is replaced where it can be, a table, an index or an event trigger made
when it is not there - and one that cannot be changed in place (another signature, a table made
by hand that is no member of the extension) is kept as it is and said in a WARNING, so the update
does not fail on a database an older script made. Nothing is dropped, no table altered: a table
that is there in the shape an older version gave it - a column of this version's missing, another
primary key - stays so, and the block after its own says that in a WARNING too. A WARNING, not a
NOTICE: PostgreSQL runs an extension's scripts with client_min_messages at warning, and a NOTICE
would reach nobody.

One thing does stop the update, with the database left at the version it had: a C function whose
symbol the library PostgreSQL has loaded does not have (or no library at all). That is an older
library still in memory - the files were replaced and PostgreSQL not restarted - and an object
kept as it is for that reason would never be made: the database would be at this version, and the
update has nothing to do in one that is. Restart PostgreSQL, run the update again.

The scripts are the same whatever version they start from - they are written from this version's
script alone - and carry its checksum (fnv1a64), which yt-core-proto's tests compare: a script
changed without `make update-sql` fails them.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TAG = "$yt_update$"


def fnv1a64(data: bytes) -> str:
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def statements(sql: str):
    """The script's top-level statements, each without the comments before it and without its
    `;`. What is inside a statement - comments, quoted text, a dollar-quoted body - stays as it is."""
    out, buf, i, n = [], [], 0, len(sql)
    started = False  # something of a statement is in buf
    while i < n:
        c = sql[i]
        if sql.startswith("--", i):
            j = sql.find("\n", i)
            j = n if j < 0 else j
            if started:
                buf.append(sql[i:j])
            i = j
        elif sql.startswith("/*", i):
            j = sql.find("*/", i)
            j = n if j < 0 else j + 2
            if started:
                buf.append(sql[i:j])
            i = j
        elif c == "\\" and not started:  # a psql line: \echo ... \quit
            j = sql.find("\n", i)
            i = n if j < 0 else j
        elif c == "'" or c == '"':
            j = quoted_end(sql, i)
            buf.append(sql[i:j + 1])
            started = True
            i = j + 1
        elif c == "$":
            m = None if i > 0 and (sql[i - 1].isalnum() or sql[i - 1] == "_") else DOLLAR.match(sql, i)
            if m:
                tag = m.group(0)
                j = sql.find(tag, i + len(tag))
                if j < 0:
                    sys.exit(f"update-sql.py: {tag} opened and never closed")
                buf.append(sql[i:j + len(tag)])
                started = True
                i = j + len(tag)
            else:
                buf.append(c)
                started = True
                i += 1
        elif c == ";":
            out.append("".join(buf).strip())
            buf, started = [], False
            i += 1
        else:
            if not c.isspace():
                started = True
            if started:
                buf.append(c)
            i += 1
    if "".join(buf).strip():
        sys.exit("update-sql.py: the script ends inside a statement: " + "".join(buf).strip()[:80])
    return out


# a dollar quote's tag: $$ or $name$ (a name does not start with a digit: $1 is a parameter)
DOLLAR = re.compile(r"\$(?:[A-Za-z_][A-Za-z_0-9]*)?\$")


def quoted_end(sql: str, i: int) -> int:
    """The index of the quote that closes the one at `i`: a 'string' (a doubled quote inside it
    is one quote; after E, a backslash takes the next character with it) or a "quoted name"."""
    q, n = sql[i], len(sql)
    escapes = q == "'" and i > 0 and sql[i - 1] in "eE" and not (i > 1 and (sql[i - 2].isalnum() or sql[i - 2] == "_"))
    j = i + 1
    while j < n:
        if escapes and sql[j] == "\\":
            j += 2
            continue
        if sql[j] == q:
            if j + 1 < n and sql[j + 1] == q:
                j += 2
                continue
            return j
        j += 1
    sys.exit(f"update-sql.py: {q} opened and never closed: {sql[i:i + 60]!r}")


def bare(sql: str) -> str:
    """`sql` without its comments; what is quoted stays as it is."""
    out, i, n = [], 0, len(sql)
    while i < n:
        c = sql[i]
        if sql.startswith("--", i):
            j = sql.find("\n", i)
            i = n if j < 0 else j
        elif sql.startswith("/*", i):
            j = sql.find("*/", i)
            i = n if j < 0 else j + 2
            out.append(" ")
        elif c == "'" or c == '"':
            j = quoted_end(sql, i)
            out.append(sql[i:j + 1])
            i = j + 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def top_level(body: str, sep: str = ","):
    """`body` cut at each `sep` that is inside no parenthesis and no quote."""
    out, buf, depth, i, n = [], [], 0, 0, len(body)
    while i < n:
        c = body[i]
        if c == "'" or c == '"':
            j = quoted_end(body, i)
            buf.append(body[i:j + 1])
            i = j + 1
            continue
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        if c == sep and depth == 0:
            out.append("".join(buf).strip())
            buf = []
        else:
            buf.append(c)
        i += 1
    if "".join(buf).strip():
        out.append("".join(buf).strip())
    return out


def name_of(word: str) -> str:
    """An identifier as the catalog has it: a quoted one as written, another in lower case."""
    return word[1:-1].replace('""', '"') if word.startswith('"') else word.lower()


def first_word(item: str) -> str:
    """The word `item` starts with: a "quoted name" whole, whatever is inside it."""
    item = item.strip()
    if item.startswith('"'):
        return item[:quoted_end(item, 0) + 1]
    return item.split()[0] if item.split() else ""


def table_shape(stmt: str):
    """A CREATE TABLE's columns and the columns of its primary key (None: it has none), as the
    catalog names them - what a table that is already there is compared with."""
    text = bare(stmt)
    open_at = text.find("(")
    if open_at < 0:
        sys.exit("update-sql.py: a CREATE TABLE with no column list:\n" + stmt[:200])
    depth, close_at, i = 0, -1, open_at
    while i < len(text):
        c = text[i]
        if c == "'" or c == '"':
            i = quoted_end(text, i) + 1
            continue
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                close_at = i
                break
        i += 1
    if close_at < 0:
        sys.exit("update-sql.py: a CREATE TABLE whose column list never closes:\n" + stmt[:200])
    columns, key = [], None
    for item in top_level(text[open_at + 1:close_at]):
        word = first_word(item)
        if not word:
            continue
        first = word.upper()
        m = re.search(r"(?is)\bPRIMARY\s+KEY\s*\((.*?)\)\s*(?:$|[A-Za-z])", item)
        if first in ("PRIMARY", "CONSTRAINT") and m:
            key = [name_of(first_word(c)) for c in top_level(m.group(1))]
        if first in ("PRIMARY", "UNIQUE", "CHECK", "FOREIGN", "CONSTRAINT", "EXCLUDE", "LIKE"):
            continue
        columns.append(name_of(word))
        if re.search(r"(?i)\bPRIMARY\s+KEY\b", item):
            key = [name_of(word)]
    if not columns:
        sys.exit("update-sql.py: a CREATE TABLE this script reads no column of:\n" + stmt[:200])
    return columns, key


def sql_text(s: str) -> str:
    return "'" + s.replace("'", "''") + "'"


def shape_check(table: str, stmt: str, version: str) -> str:
    """The block after a table's own: a table that was there already is not altered, so one in
    the shape an older version gave it is said - what it lacks, what it is keyed by."""
    columns, key = table_shape(stmt)
    keyed = ", ".join(key) if key else ""
    return (f"DO {TAG}\nDECLARE lacks text; keyed text;\nBEGIN\n"
            f"    SELECT string_agg(c, ', ') INTO lacks FROM unnest(ARRAY[{', '.join(sql_text(c) for c in columns)}]) c\n"
            f"     WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_attribute a\n"
            f"                        WHERE a.attrelid = {sql_text(table)}::regclass AND a.attname = c AND NOT a.attisdropped);\n"
            f"    SELECT coalesce(string_agg(a.attname, ', ' ORDER BY k.n), '') INTO keyed\n"
            f"      FROM pg_catalog.pg_index i CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY k(attnum, n)\n"
            f"      JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum\n"
            f"     WHERE i.indrelid = {sql_text(table)}::regclass AND i.indisprimary;\n"
            f"    IF lacks IS NOT NULL OR keyed <> {sql_text(keyed)} THEN\n"
            f"        RAISE WARNING 'yt_cluster update to {version}: % has the shape an older version gave it "
            f"(columns it lacks: %; primary key: %, this version''s: %) - an update alters no table',\n"
            f"            {sql_text('table ' + table)}, coalesce(lacks, 'none'), "
            f"coalesce(nullif(keyed, ''), 'none'), {sql_text(keyed if keyed else 'none')};\n"
            f"    END IF;\n"
            f"EXCEPTION WHEN OTHERS THEN\n"
            f"    RAISE WARNING 'yt_cluster update to {version}: % is not compared with this version''s: %', "
            f"{sql_text('table ' + table)}, SQLERRM;\n"
            f"END {TAG};\n")


def squeeze(s: str) -> str:
    return re.sub(r"\s+", " ", s).strip()


def block(what: str, body: str, version: str, library: bool = False) -> str:
    """One statement of the script in a block that keeps what it cannot change. The `;` is on a
    line of its own: a statement that ends in a `--` comment would take it into the comment.
    `library`: the statement binds a C symbol - see the module's note on what stops the update."""
    stop = ""
    if library:
        stop = ("    WHEN undefined_function OR undefined_file THEN\n"
                f"        RAISE EXCEPTION 'yt_cluster update to {version}: the yt_cluster library PostgreSQL has loaded "
                "is not this version''s (%): restart PostgreSQL, then run ALTER EXTENSION yt_cluster UPDATE again', SQLERRM;\n")
    return (f"DO {TAG} BEGIN\n{body}\n;\nEXCEPTION\n{stop}    WHEN OTHERS THEN\n"
            f"        RAISE WARNING 'yt_cluster update to {version}: % is kept as it is: %', "
            f"{sql_text(what)}, SQLERRM;\nEND {TAG};\n")


def update_of(stmt: str, version: str) -> str:
    """One statement of the script as the update runs it."""
    if TAG in stmt:
        sys.exit(f"update-sql.py: a statement holds {TAG}")
    head = squeeze(stmt[:200]).upper()
    m = re.match(r"(?is)\s*CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION\s+([^\s(]+\s*\([^)]*\))", stmt)
    if m:
        body = re.sub(r"(?is)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION", "CREATE OR REPLACE FUNCTION", stmt, count=1)
        sig = squeeze(m.group(1))
        in_c = re.search(r"(?i)\bLANGUAGE\s+c\b", bare(stmt)) is not None
        return block("function " + (sig if len(sig) <= 70 else sig[:67] + "..."), body, version, library=in_c)
    m = re.match(r"(?is)\s*CREATE\s+(?:OR\s+REPLACE\s+)?VIEW\s+(\S+)", stmt)
    if m:
        body = re.sub(r"(?is)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?VIEW", "CREATE OR REPLACE VIEW", stmt, count=1)
        return block("view " + m.group(1), body, version)
    m = re.match(r"(?is)\s*CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\S+)", stmt)
    if m:
        return block("table " + m.group(1), stmt, version) + "\n" + shape_check(m.group(1), stmt, version)
    m = re.match(r"(?is)\s*CREATE\s+(?:UNIQUE\s+)?INDEX\s+IF\s+NOT\s+EXISTS\s+(\S+)", stmt)
    if m:
        return block("index " + m.group(1), stmt, version)
    m = re.match(r"(?is)\s*CREATE\s+EVENT\s+TRIGGER\s+(\S+)", stmt)
    if m:
        name = m.group(1)
        body = (f"IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_event_trigger WHERE evtname = {sql_text(name)}) THEN\n"
                f"{stmt}\n;\nEND IF")
        return block("event trigger " + name, body, version)
    m = re.match(r"(?is)\s*SELECT\s+(pg_catalog\.pg_extension_config_dump\s*\(.*\))\s*$", stmt)
    if m:
        return block("the dump of " + squeeze(m.group(1)), "PERFORM " + m.group(1), version)
    if head.startswith(("REVOKE ", "GRANT ", "COMMENT ")):
        return block(squeeze(stmt)[:70], stmt, version)
    if head.startswith("DO "):
        return stmt + "\n;\n"  # a block of the script's own, with its own handler
    sys.exit("update-sql.py: a statement this script does not know how to run in an update - "
             "teach it, or the update would leave it out:\n" + stmt[:300])


def make(version: str, earlier: str, script: bytes) -> str:
    name = f"yt_cluster--{version}.sql"
    out = [
        "-- SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent\n",
        f"-- yt_cluster {earlier} -> {version}: what ALTER EXTENSION yt_cluster UPDATE runs in a database that has {earlier}.\n",
        f"-- Made by update-sql.py from {name} (fnv1a64 {fnv1a64(script)}): never edited - `make update-sql`.\n",
        "--\n",
        f"-- It brings the database to what CREATE EXTENSION makes at {version}, an object at a time: a function or a\n",
        "-- view replaced where it can be, a table, an index or an event trigger made when it is not there. One that\n",
        "-- cannot be changed in place is kept as it is and said in a WARNING - the update does not fail on it.\n",
        "-- Nothing is dropped and no table is altered: one in the shape an older version gave it is said too.\n",
        "-- A C function the loaded library does not have stops the update, the database left at its version:\n",
        "-- that library is an older one still in memory - restart PostgreSQL and run the update again.\n\n",
        f"\\echo Use \"ALTER EXTENSION yt_cluster UPDATE TO '{version}'\" to load this file. \\quit\n\n",
    ]
    for s in statements(script.decode("utf-8")):
        out.append(update_of(s, version))
        out.append("\n")
    return "".join(out)


def main() -> int:
    version = ".".join(open(os.path.join(HERE, "..", "VERSION")).read().strip().split(".")[:2])
    script_path = os.path.join(HERE, f"yt_cluster--{version}.sql")
    script = open(script_path, "rb").read()
    earlier = [l.strip() for l in open(os.path.join(HERE, "previous-versions.txt")) if l.strip() and not l.startswith("#")]
    check, bad = "--check" in sys.argv, 0
    for e in earlier:
        if e == version:
            sys.exit(f"update-sql.py: previous-versions.txt lists {version}, the version of ../VERSION")
        path = os.path.join(HERE, f"yt_cluster--{e}--{version}.sql")
        text = make(version, e, script)
        have = open(path, encoding="utf-8").read() if os.path.exists(path) else None
        if check:
            if have != text:
                print(f"{os.path.basename(path)}: not what update-sql.py writes from {os.path.basename(script_path)} - make update-sql")
                bad += 1
        elif have != text:
            open(path, "w", encoding="utf-8").write(text)
            print("wrote", os.path.basename(path))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
