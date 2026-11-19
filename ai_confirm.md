# Is it true? The comparison table of dbinvent.com, cross-referenced

The comparison table on https://www.dbinvent.com/ ("Why not the thing you already tried"): for seven PostgreSQL
clustering options and 16 capabilities, and its hybrid table below it, 137 statements in all, as of November 2026. Each one comes
with the page that states it and the sentence quoted from that page: YT Cluster as DBinvent states
and explains it on dbinvent.com, every other project as its own documentation states it, and
third-party sources where there are some. It cross-references what each project documents - and
what it does not; it is not a ranking and not a benchmark.

Review it, then give the whole file to the AI you prefer. The rules and the one question it is to
answer are at the end.

## How a statement reads

Every statement is a block, numbered by project (A to G) and capability (1 to 16), such as C4:

- Project and Capability: whose statement it is, and about what.
- Mark: Yes, Partly or No - see the rules at the end.
- Note: a short hint, not a full description.
- Source and Quote: the page that states it and the sentence copied from that page; a statement
  can have more than one.

A "No" seldom has a page that states an absence: its source is the project's page for the closest
feature, or its overview, which shows what the project does instead.

## The capabilities

1. Smart 2PC - a participant node that fails does not ruin cluster writes
2. Core component open source (MIT)
3. Every node accepts writes
4. Keeps writing when a node fails
5. A site cut off from the network keeps writing
6. Consistency chosen per table
7. Unmodified PostgreSQL engine
8. No data migration to start
9. Sharding by rules: overlapping shards, availability zones (AZ)
10. MPP analytics on the same data
11. Serverless Python, KV and queue, built in
12. Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
13. The cluster carries the migration scripts to its nodes
14. A stopped schema change picks up by itself
15. A CI told when a schema change is on every node
16. Leave any time, back to plain PostgreSQL

## The statements, by project

### A. YT Cluster

DBinvent's PostgreSQL cluster - as DBinvent states and explains it on dbinvent.com, with its public test suite at https://github.com/DBinvent/yt-cluster-tests.

#### A1

- Project: YT Cluster
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: Yes
- Note: the party is chosen among the nodes up: a failed node is left out and catches up when back; one that fails inside a transaction rolls back that one only
- Source: https://www.dbinvent.com/cluster/guide/docker
  Quote: "A node going down stops nothing: the others go on taking writes, and it catches up when it is back."
- Source: https://www.dbinvent.com/cluster/guide/configuration#failover-settings
  Quote: "YT Cluster allows user to specify how many cluster nodes must synchronously repeat the transaction before it is considered successful."

#### A2

- Project: YT Cluster
- Capability: Core component open source (MIT)
- Mark: Yes
- Note: the node's core, yt-lite: the protocol, the PostgreSQL extension, the link code and the lite node; the cluster's own server is not
- Source: https://www.dbinvent.com/cluster/
  Quote: "Open source node - yt-lite, the open-source version of the cluster node, under the MIT license: a YT node for one database, linked to a YT Cluster above it."
- Source: https://www.dbinvent.com/cluster/guide/licensing#open-core
  Quote: "The core of YT Cluster - the PostgreSQL extension, the protocol and yt-lite, the node for one database - is open source under the MIT license."
- Source: https://www.dbinvent.com/cluster/articles/rethinking-client-server
  Quote: "The YT Cluster project itself has run for 6 years, and its core is open source now, to give a better experience through embedded solutions."
- Source: https://www.dbinvent.com/cluster/articles/rethinking-client-server
  Quote: "The PostgreSQL extension under yt-lite is the one a full node runs."
- Source: https://www.dbinvent.com/cluster/articles/rethinking-client-server
  Quote: "So the node in an application and the node in the data centre cannot drift apart: what they share is built from one source, yt-cluster-lite, under the MIT license."
- Source: https://github.com/DBinvent/yt-cluster-lite
  Quote: "YT Cluster lite is the open part of YT Cluster, under the MIT license."

#### A3

- Project: YT Cluster
- Capability: Every node accepts writes
- Mark: Yes
- Note: every node
- Source: https://www.dbinvent.com/cluster/articles/fault-tolerance
  Quote: "Every node accepts transactions, so a node becoming unavailable removes capacity rather than availability."

#### A4

- Project: YT Cluster
- Capability: Keeps writing when a node fails
- Mark: Yes
- Note: no promotion step; built-in DNS answers an A record per node
- Source: https://www.dbinvent.com/cluster/articles/fault-tolerance
  Quote: "There is no promotion window, no split-brain drill, and no failover runbook to keep current - the remaining nodes were already serving writes."
- Source: https://www.dbinvent.com/cluster/guide/architecture#dns
  Quote: "Any node answers any query with the same reply: an A record for each node that is up right now, built from the peer list the cluster protocol keeps anyway."

#### A5

- Project: YT Cluster
- Capability: A site cut off from the network keeps writing
- Mark: Yes
- Note: as a linked cluster of its own, by rules set per table
- Source: https://www.dbinvent.com/cluster/articles/disconnected-operations#what-dbinvent-does
  Quote: "Every node is a complete, ACID PostgreSQL database. It reads and writes at full speed with no connectivity whatsoever"

#### A6

- Project: YT Cluster
- Capability: Consistency chosen per table
- Mark: Yes
- Note: nodes to confirm, conflict rule
- Source: https://www.dbinvent.com/cluster/guide/configuration#failover-settings
  Quote: "The number of nodes can be set both for the entire cluster and for individual tables."
- Source: https://www.dbinvent.com/cluster/articles/disjoint-json
  Quote: "Per-table merge for jsonb row updates is configured through CfgTabl."

#### A7

- Project: YT Cluster
- Capability: Unmodified PostgreSQL engine
- Mark: Yes
- Note: plus an extension
- Source: https://www.dbinvent.com/cluster/guide/architecture#extension
  Quote: "The extension works inside the PostgreSQL server address space and communicates only with the YT-Server application of the respective node."

#### A8

- Project: YT Cluster
- Capability: No data migration to start
- Mark: Yes
- Note: your database becomes a node
- Source: https://www.dbinvent.com/cluster/guide/migration#restart
  Quote: "The extension needs no preloading to replicate."
- Source: https://www.dbinvent.com/cluster/articles/easy-scalability
  Quote: "Installing the extension makes it a node in place - nothing is dumped, copied or reloaded - and removing it leaves plain PostgreSQL with the data still there."

#### A9

- Project: YT Cluster
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: Yes
- Note: shards are optional rules, set per table
- Source: https://www.dbinvent.com/cluster/articles/where-rows-live#shape-as-data-zones-tiers-and-three-places-a-row-can-leak
  Quote: "A node carries an availability zone - a string, nothing more."
- Source: https://www.dbinvent.com/cluster/guide/architecture#topology
  Quote: "Replication scope and row placement are rules rather than a deployed shape."

#### A10

- Project: YT Cluster
- Capability: MPP analytics on the same data
- Mark: Yes
- Note: DataFusion beside each node, given the data layout by ytserv; tested in public
- Source: https://www.dbinvent.com/cluster/guide/architecture#mpp
  Quote: "Analytical queries are planned and executed by Apache DataFusion across the declared topology, reaching tables already marked for cluster synchronization without copying them anywhere first."

#### A11

- Project: YT Cluster
- Capability: Serverless Python, KV and queue, built in
- Mark: Yes
- Note: server functions, etcd API
- Source: https://www.dbinvent.com/cluster/guide/architecture#functions
  Quote: "RPPD is the server-function runtime: Python triggered by insert, update and delete, deployed as functions rather than as services"
- Source: https://www.dbinvent.com/cluster/guide/architecture#etcd
  Quote: "A node can expose a standard etcd API v3 key-value surface"
- Source: https://www.dbinvent.com/cluster/guide/architecture#etcd
  Quote: "The queue is built on it: a named queue with ordering and delivery guarantees"

#### A12

- Project: YT Cluster
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: Yes
- Note: node by node or all at once, from one row
- Source: https://www.dbinvent.com/cluster/guide/schema-rollout#how
  Quote: "Nodes do it in turn on purpose: every node not migrating is up and serving"
- Source: https://www.dbinvent.com/cluster/guide/schema-rollout#at-once
  Quote: "Optional: the worker tells every node at the same time."

#### A13

- Project: YT Cluster
- Capability: The cluster carries the migration scripts to its nodes
- Mark: Yes
- Note: a zip or a folder, checked by MD5
- Source: https://www.dbinvent.com/cluster/guide/schema-rollout#scripts
  Quote: "The archive on the node you start from is carried to every other node over gRPC, and checked by its MD5."

#### A14

- Project: YT Cluster
- Capability: A stopped schema change picks up by itself
- Mark: Yes
- Note: the leader takes it over
- Source: https://www.dbinvent.com/cluster/guide/schema-rollout#failure
  Quote: "The leader, also a new one after an election, asks every node about a roll in progress or abandoned, and takes it over with itself as the origin."

#### A15

- Project: YT Cluster
- Capability: A CI told when a schema change is on every node
- Mark: Yes
- Note: HTTPS or Kafka message
- Source: https://www.dbinvent.com/cluster/guide/schema-rollout#ci
  Quote: "The etcds bridge sends it on, to Kafka or over HTTPS to the CI"

#### A16

- Project: YT Cluster
- Capability: Leave any time, back to plain PostgreSQL
- Mark: Yes
- Note: tested in public
- Source: https://www.dbinvent.com/cluster/guide/migration#back
  Quote: "The way out has no downtime either: the same in reverse, one node at a time, while the others keep writing."
- Source: https://www.dbinvent.com/cluster/guide/migration#tested
  Quote: "This procedure is a public test suite on the nightly run"

### B. Patroni + etcd

Plain PostgreSQL with streaming replication, Patroni as its high-availability manager and etcd as Patroni's store - as the Patroni and PostgreSQL documentation states.

#### B1

- Project: Patroni + etcd
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: Partly
- Note: one primary: a failed synchronous standby blocks commits until Patroni goes standalone - in strict mode until one is back
- Source: https://patroni.readthedocs.io/en/latest/replication_modes.html
  Quote: "commits will block until next iteration of Patroni runs and switches the primary to standalone mode."

#### B2

- Project: Patroni + etcd
- Capability: Core component open source (MIT)
- Mark: Yes
- Note: Patroni MIT; etcd Apache 2.0; PostgreSQL its own license, like MIT
- Source: https://github.com/patroni/patroni/blob/master/LICENSE
  Quote: "The MIT License (MIT)"
- Source: https://github.com/etcd-io/etcd
  Quote: "etcd is under the Apache 2.0 license."
- Source: https://www.postgresql.org/about/licence/
  Quote: "PostgreSQL is released under the PostgreSQL License, a liberal Open Source license, similar to the BSD or MIT licenses."

#### B3

- Project: Patroni + etcd
- Capability: Every node accepts writes
- Mark: No
- Note: one primary
- Source: https://www.postgresql.org/docs/current/hot-standby.html
  Quote: "All such connections are strictly read-only; not even temporary tables may be written."

#### B4

- Project: Patroni + etcd
- Capability: Keeps writing when a node fails
- Mark: Partly
- Note: after a promotion
- Source: https://patroni.readthedocs.io/en/latest/faq.html
  Quote: "Patroni automatic failover is based on what we call leader race."

#### B5

- Project: Patroni + etcd
- Capability: A site cut off from the network keeps writing
- Mark: No
- Note: the etcd majority's side only
- Source: https://patroni.readthedocs.io/en/latest/ha_multi_dc.html
  Quote: "Automatic promotion is not possible, because DC2 will never able to figure out the state of DC1."

#### B6

- Project: Patroni + etcd
- Capability: Consistency chosen per table
- Mark: Partly
- Note: per transaction, via synchronous_commit
- Source: https://patroni.readthedocs.io/en/latest/replication_modes.html
  Quote: "Clients can change the behavior per transaction using PostgreSQL's synchronous_commit setting."

#### B7

- Project: Patroni + etcd
- Capability: Unmodified PostgreSQL engine
- Mark: Yes
- Note: (none)
- Source: https://patroni.readthedocs.io/en/latest/
  Quote: "Patroni is a template for high availability (HA) PostgreSQL solutions using Python."
- Source: https://patroni.readthedocs.io/en/latest/existing_data.html
  Quote: "This section describes the process for converting a standalone PostgreSQL instance into a Patroni cluster."

#### B8

- Project: Patroni + etcd
- Capability: No data migration to start
- Mark: Yes
- Note: (none)
- Source: https://patroni.readthedocs.io/en/latest/faq.html
  Quote: "Yes, it can! (on taking over an existing Postgres cluster; see Convert a Standalone to a Patroni Cluster)"
- Source: https://patroni.readthedocs.io/en/latest/existing_data.html
  Quote: "This section describes the process for converting a standalone PostgreSQL instance into a Patroni cluster."

#### B9

- Project: Patroni + etcd
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: No
- Note: no sharding
- Source: https://www.postgresql.org/docs/current/high-availability.html
  Quote: "or to allow several computers to serve the same data (load balancing)"

#### B10

- Project: Patroni + etcd
- Capability: MPP analytics on the same data
- Mark: No
- Note: (none)
- Source: https://www.postgresql.org/docs/current/high-availability.html
  Quote: "or to allow several computers to serve the same data (load balancing)"

#### B11

- Project: Patroni + etcd
- Capability: Serverless Python, KV and queue, built in
- Mark: No
- Note: PL/Python in a session; no KV or queue
- Source: https://www.postgresql.org/docs/current/plpython.html
  Quote: "The PL/Python procedural language allows PostgreSQL functions and procedures to be written in the Python language."

#### B12

- Project: Patroni + etcd
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: No
- Note: replicated from the primary; your tool
- Source: https://www.postgresql.org/docs/current/logical-replication-restrictions.html
  Quote: "The database schema and DDL commands are not replicated."

#### B13

- Project: Patroni + etcd
- Capability: The cluster carries the migration scripts to its nodes
- Mark: No
- Note: (none)
- Source: https://patroni.readthedocs.io/en/latest/
  Quote: "Patroni is a template for high availability (HA) PostgreSQL solutions using Python."

#### B14

- Project: Patroni + etcd
- Capability: A stopped schema change picks up by itself
- Mark: No
- Note: (none)
- Source: https://patroni.readthedocs.io/en/latest/
  Quote: "Patroni is a template for high availability (HA) PostgreSQL solutions using Python."

#### B15

- Project: Patroni + etcd
- Capability: A CI told when a schema change is on every node
- Mark: No
- Note: (none)
- Source: https://patroni.readthedocs.io/en/latest/
  Quote: "Patroni is a template for high availability (HA) PostgreSQL solutions using Python."

#### B16

- Project: Patroni + etcd
- Capability: Leave any time, back to plain PostgreSQL
- Mark: Yes
- Note: it is plain PostgreSQL
- Source: https://patroni.readthedocs.io/en/latest/
  Quote: "Patroni is a template for high availability (HA) PostgreSQL solutions using Python."
- Source: https://patroni.readthedocs.io/en/latest/existing_data.html
  Quote: "This section describes the process for converting a standalone PostgreSQL instance into a Patroni cluster."
- Source: https://patroni.readthedocs.io/en/latest/pause.html
  Quote: "When Patroni is stopped, it does not try to stop the Postgres instance it is managing."

Third-party sources on Patroni + etcd:

- Source: https://docs.percona.com/postgresql/16/solutions/patroni-info.html
  Quote: "The etcd layer uses the Raft consensus algorithm to allow only one node to acquire the leader key."

### C. CockroachDB

Cockroach Labs' distributed SQL database, PostgreSQL wire-compatible - as its documentation states.

#### C1

- Project: CockroachDB
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: No
- Note: no two-phase commit: dropped for its own Parallel Commits; consensus per range lets a minority of replicas fail
- Source: https://www.cockroachlabs.com/blog/parallel-commits/
  Quote: "we had to throw away two-phase commit and rework how transactions in CockroachDB arrive at a committed state."
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/replication-layer
  Quote: "For example, with 3x replication, one failure can be tolerated; with 5x replication, two failures, and so on."

#### C2

- Project: CockroachDB
- Capability: Core component open source (MIT)
- Mark: No
- Note: source available, under its own license
- Source: https://docs.cockroachlabs.com/docs/stable/licensing-faqs
  Quote: "are made available under the CockroachDB Software License"

#### C3

- Project: CockroachDB
- Capability: Every node accepts writes
- Mark: Yes
- Note: any node
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/overview
  Quote: "Thanks to the symmetrical behavior of all nodes in a cluster, you can send SQL requests to any node"

#### C4

- Project: CockroachDB
- Capability: Keeps writing when a node fails
- Mark: Yes
- Note: ranges re-elect
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/replication-layer
  Quote: "a Raft election is initiated to establish a new leader for the range"

#### C5

- Project: CockroachDB
- Capability: A site cut off from the network keeps writing
- Mark: No
- Note: the majority side only
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/replication-layer
  Quote: "a majority of the replica nodes must be able to communicate with each other to coordinate the write"

#### C6

- Project: CockroachDB
- Capability: Consistency chosen per table
- Mark: Partly
- Note: a commit waits for a quorum of the table's own replication factor; one isolation level for all
- Source: https://docs.cockroachlabs.com/docs/stable/configure-replication-zones
  Quote: "You can add replication zones for specific tables."
- Source: https://docs.cockroachlabs.com/docs/stable/configure-replication-zones
  Quote: "The number of replicas in the zone, also called the "replication factor"."
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/replication-layer
  Quote: "require that a quorum of replicas agrees on any changes to a range before those changes are committed"

#### C7

- Project: CockroachDB
- Capability: Unmodified PostgreSQL engine
- Mark: No
- Note: its own engine
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/overview
  Quote: "data is written to and read from disk using an efficient storage engine"

#### C8

- Project: CockroachDB
- Capability: No data migration to start
- Mark: No
- Note: data moves into its engine
- Source: https://docs.cockroachlabs.com/docs/stable/migration-overview
  Quote: "A migration involves transferring data from a pre-existing source database onto a target CockroachDB cluster"

#### C9

- Project: CockroachDB
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: Partly
- Note: automatic ranges; zone configs
- Source: https://docs.cockroachlabs.com/docs/stable/configure-replication-zones
  Quote: "Replication zones give you the power to control what data goes where in your CockroachDB cluster"

#### C10

- Project: CockroachDB
- Capability: MPP analytics on the same data
- Mark: Partly
- Note: distributed SQL, not an analytics engine
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/overview
  Quote: "developers interact with CockroachDB through a PostgreSQL-compatible SQL API"

#### C11

- Project: CockroachDB
- Capability: Serverless Python, KV and queue, built in
- Mark: No
- Note: (none)
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/overview
  Quote: "developers interact with CockroachDB through a PostgreSQL-compatible SQL API"

#### C12

- Project: CockroachDB
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: Partly
- Note: online DDL, no lock; versioned scripts are your tool
- Source: https://docs.cockroachlabs.com/docs/stable/online-schema-changes
  Quote: "The schema change runs as a background job without holding locks on the underlying table data"

#### C13

- Project: CockroachDB
- Capability: The cluster carries the migration scripts to its nodes
- Mark: No
- Note: (none)
- Source: https://docs.cockroachlabs.com/docs/stable/architecture/overview
  Quote: "developers interact with CockroachDB through a PostgreSQL-compatible SQL API"

#### C14

- Project: CockroachDB
- Capability: A stopped schema change picks up by itself
- Mark: Yes
- Note: schema-change jobs resume
- Source: https://docs.cockroachlabs.com/docs/stable/show-jobs
  Quote: "automatically retries jobs that fail due to retry errors or job coordination failures"

#### C15

- Project: CockroachDB
- Capability: A CI told when a schema change is on every node
- Mark: Partly
- Note: poll the job
- Source: https://docs.cockroachlabs.com/docs/stable/show-jobs
  Quote: "To wait for a job to finish, use SHOW JOB WHEN COMPLETE"

#### C16

- Project: CockroachDB
- Capability: Leave any time, back to plain PostgreSQL
- Mark: Partly
- Note: export, or replicate back with MOLT
- Source: https://docs.cockroachlabs.com/docs/stable/migration-overview
  Quote: "Failback replication from CockroachDB back to source databases"

Third-party sources on CockroachDB:

- Source: https://jepsen.io/analyses/cockroachdb-beta-20160829
  Quote: "CockroachDB does not provide external consistency (e.g. full-database linearizability, or strict serializability)"
- Source: https://en.wikipedia.org/wiki/CockroachDB
  Quote: "distributed SQL database management system"

### D. Greenplum

The MPP data warehouse (Broadcom), built on PostgreSQL; its source closed in May 2024 - as its documentation states.

#### D1

- Project: Greenplum
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: Partly
- Note: a segment that fails rolls back the transactions in progress; its mirror takes the next ones
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-highavail-topics-g-overview-of-segment-mirroring.html
  Quote: "Transactions in progress when the failure occurred roll back and must be restarted."

#### D2

- Project: Greenplum
- Capability: Core component open source (MIT)
- Mark: No
- Note: none kept in the open: its repository archived, read-only since May 2024 (that last version Apache 2.0)
- Source: https://github.com/greenplum-db/gpdb-archive
  Quote: "This repository was archived by the owner on May 24, 2024."
- Source: https://github.com/greenplum-db/gpdb-archive
  Quote: "The Greenplum project is released under the Apache 2 license."

#### D3

- Project: Greenplum
- Capability: Every node accepts writes
- Mark: No
- Note: through the coordinator
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "It is the database instance to which clients connect and submit SQL statements"

#### D4

- Project: Greenplum
- Capability: Keeps writing when a node fails
- Mark: Partly
- Note: mirrors take over, the coordinator by hand
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-about_ha.html
  Quote: "automatically fail over to the mirror segment instance if a primary segment instance becomes unavailable"

#### D5

- Project: Greenplum
- Capability: A site cut off from the network keeps writing
- Mark: No
- Note: (none)
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-about_ha.html
  Quote: "serves as a warm standby in the event that the primary coordinator host becomes non-operational"

#### D6

- Project: Greenplum
- Capability: Consistency chosen per table
- Mark: Partly
- Note: per transaction: isolation level; mirrors alike for all
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-about_mvcc.html
  Quote: "The default isolation level in Greenplum Database is READ COMMITTED"

#### D7

- Project: Greenplum
- Capability: Unmodified PostgreSQL engine
- Mark: No
- Note: PostgreSQL 12 with its internals modified
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "It is based on PostgreSQL 12, and in most cases is very similar to PostgreSQL with regard to SQL support"
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "The internals of PostgreSQL have been modified or supplemented to support the parallel structure of Greenplum Database."

#### D8

- Project: Greenplum
- Capability: No data migration to start
- Mark: No
- Note: loaded into the warehouse
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-external-g-external-tables.html
  Quote: "to load or unload data"

#### D9

- Project: Greenplum
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: No
- Note: a distribution key per table
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-ddl-ddl-table.html
  Quote: "To ensure even distribution of hash-distributed data, choose a distribution key that is unique for each record"

#### D10

- Project: Greenplum
- Capability: MPP analytics on the same data
- Mark: Yes
- Note: what it is built for
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/ref_guide-feature_summary.html
  Quote: "a parallel, shared-nothing database system"

#### D11

- Project: Greenplum
- Capability: Serverless Python, KV and queue, built in
- Mark: No
- Note: PL/Python in a session; no KV or queue
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/ref_guide-feature_summary.html
  Quote: "also supports function definitions written in Python, Perl, Java, and R"

#### D12

- Project: Greenplum
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: No
- Note: DDL through the coordinator; your tool
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "distributes workloads among segments"

#### D13

- Project: Greenplum
- Capability: The cluster carries the migration scripts to its nodes
- Mark: No
- Note: (none)
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "distributes workloads among segments"

#### D14

- Project: Greenplum
- Capability: A stopped schema change picks up by itself
- Mark: No
- Note: (none)
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "distributes workloads among segments"

#### D15

- Project: Greenplum
- Capability: A CI told when a schema change is on every node
- Mark: No
- Note: (none)
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-intro-arch_overview.html
  Quote: "distributes workloads among segments"

#### D16

- Project: Greenplum
- Capability: Leave any time, back to plain PostgreSQL
- Mark: No
- Note: export and migrate
- Source: https://techdocs.broadcom.com/us/en/vmware-tanzu/data-solutions/tanzu-greenplum/7/greenplum-database/admin_guide-external-g-external-tables.html
  Quote: "to load or unload data"

Third-party sources on Greenplum:

- Source: https://en.wikipedia.org/wiki/Greenplum
  Quote: "In May 2024, Tanzu by Broadcom made the decision to close source the Greenplum Database project"
- Source: https://cloudberry.apache.org/blog/cloudberry-database-enters-the-apache-incubator
  Quote: "original open-source Greenplum users cannot get the security or feature updates for free anymore"

### E. PgDog

The open-source PostgreSQL proxy with sharding and connection pooling - as its documentation and README state.

#### E1

- Project: PgDog
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: No
- Note: an error in phase 1 rolls the cross-shard write back; each shard is one primary
- Source: https://docs.pgdog.dev/features/sharding/2pc/
  Quote: "If an error happens before or during phase 1, the transaction will be rolled back automatically."

#### E2

- Project: PgDog
- Capability: Core component open source (MIT)
- Mark: Partly
- Note: open source, AGPL v3
- Source: https://github.com/pgdogdev/pgdog
  Quote: "PgDog is free and open source software, licensed under the AGPL v3."

#### E3

- Project: PgDog
- Capability: Every node accepts writes
- Mark: No
- Note: a single writer: one primary per shard; a read after it can hit a stale replica
- Source: https://docs.pgdog.dev/features/load-balancer/
  Quote: "For all other queries, PgDog will send them to the primary."
- Source: https://docs.pgdog.dev/features/load-balancer/
  Quote: "PgDog will send it to one of the replicas"

#### E4

- Project: PgDog
- Capability: Keeps writing when a node fails
- Mark: Partly
- Note: that shard waits for a failover
- Source: https://docs.pgdog.dev/features/load-balancer/
  Quote: "Replica lag detection and automatic traffic failover on replica promotion."

#### E5

- Project: PgDog
- Capability: A site cut off from the network keeps writing
- Mark: No
- Note: (none)
- Source: https://github.com/pgdogdev/pgdog
  Quote: "This doesn't replace tools like Patroni that actually orchestrate failovers."

#### E6

- Project: PgDog
- Capability: Consistency chosen per table
- Mark: Partly
- Note: per transaction, via PostgreSQL
- Source: https://docs.pgdog.dev/features/sharding/2pc/
  Quote: "Two-phase commit takes advantage of prepared transactions in Postgres to provide eventually consistent cross-shard writes."

#### E7

- Project: PgDog
- Capability: Unmodified PostgreSQL engine
- Mark: Yes
- Note: behind a proxy
- Source: https://github.com/pgdogdev/pgdog
  Quote: "PgDog is an application layer (OSI Level 7) load balancer for PostgreSQL."
- Source: https://docs.pgdog.dev/features/sharding/
  Quote: "Each machine runs its own, independent PostgreSQL server"

#### E8

- Project: PgDog
- Capability: No data migration to start
- Mark: Partly
- Note: to shard, data moves
- Source: https://docs.pgdog.dev/features/sharding/resharding/
  Quote: "To make this an online process, with zero downtime or data loss, PgDog hooks into the logical replication protocol"

#### E9

- Project: PgDog
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: Partly
- Note: a shard is a whole database server; list, range or hash on a key; no overlap or zones
- Source: https://docs.pgdog.dev/features/sharding/
  Quote: "Each machine runs its own, independent PostgreSQL server."
- Source: https://docs.pgdog.dev/configuration/pgdog.toml/sharded_tables/
  Quote: "By default, PgDog uses hash-based sharding, with data evenly split between shards."

#### E10

- Project: PgDog
- Capability: MPP analytics on the same data
- Mark: Partly
- Note: cross-shard queries in the proxy
- Source: https://docs.pgdog.dev/features/sharding/cross-shard-queries/
  Quote: "PgDog implements a scatter/gather query engine to fetch rows from multiple shards concurrently."

#### E11

- Project: PgDog
- Capability: Serverless Python, KV and queue, built in
- Mark: No
- Note: (none)
- Source: https://github.com/pgdogdev/pgdog
  Quote: "PgDog is an application layer (OSI Level 7) load balancer for PostgreSQL."

#### E12

- Project: PgDog
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: No
- Note: sent to the shards; your tool
- Source: https://docs.pgdog.dev/features/sharding/schema_management/migrations/
  Quote: "PgDog sends DDL queries to all shards concurrently."

#### E13

- Project: PgDog
- Capability: The cluster carries the migration scripts to its nodes
- Mark: No
- Note: (none)
- Source: https://docs.pgdog.dev/features/sharding/schema_management/migrations/
  Quote: "PgDog sends DDL queries to all shards concurrently."

#### E14

- Project: PgDog
- Capability: A stopped schema change picks up by itself
- Mark: No
- Note: (none)
- Source: https://docs.pgdog.dev/features/sharding/schema_management/migrations/
  Quote: "If an error occurs on one or more of the databases, the whole transaction will be rolled back."

#### E15

- Project: PgDog
- Capability: A CI told when a schema change is on every node
- Mark: No
- Note: (none)
- Source: https://docs.pgdog.dev/features/sharding/schema_management/migrations/
  Quote: "PgDog sends DDL queries to all shards concurrently."

#### E16

- Project: PgDog
- Capability: Leave any time, back to plain PostgreSQL
- Mark: Partly
- Note: drop the proxy; sharded data stays split
- Source: https://github.com/pgdogdev/pgdog
  Quote: "PgDog is an application layer (OSI Level 7) load balancer for PostgreSQL."
- Source: https://docs.pgdog.dev/features/sharding/resharding/
  Quote: "Resharding changes the number of shards in an existing database cluster in order to add or remove capacity."

### F. Citus

The open-source PostgreSQL extension for distributed tables, self-hosted - as its documentation, README and blog state.

#### F1

- Project: Citus
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: No
- Note: a worker is one primary for its shards: their writes fail until its standby is promoted
- Source: https://docs.citusdata.com/en/stable/admin_guide/cluster_management.html
  Quote: "Citus uses PostgreSQL streaming replication, allowing it to tolerate worker-node failures."

#### F2

- Project: Citus
- Capability: Core component open source (MIT)
- Mark: Partly
- Note: open source, AGPL v3
- Source: https://github.com/citusdata/citus
  Quote: "The Citus database is 100% open source."
- Source: https://github.com/citusdata/citus/blob/main/LICENSE
  Quote: "GNU AFFERO GENERAL PUBLIC LICENSE"

#### F3

- Project: Citus
- Capability: Every node accepts writes
- Mark: Partly
- Note: since Citus 11 any node takes the query and passes a write to the shard's one primary
- Source: https://docs.citusdata.com/en/stable/get_started/concepts.html
  Quote: "Applications send their queries to the coordinator node which relays it to the relevant workers and accumulates the results."
- Source: https://www.citusdata.com/blog/2022/06/17/citus-11-goes-fully-open-source/
  Quote: "you can now always run distributed queries from any node in the cluster"

#### F4

- Project: Citus
- Capability: Keeps writing when a node fails
- Mark: Partly
- Note: that shard waits for a failover
- Source: https://docs.citusdata.com/en/v12.1/admin_guide/cluster_management.html
  Quote: "Citus uses PostgreSQL streaming replication, allowing it to tolerate worker-node failures."

#### F5

- Project: Citus
- Capability: A site cut off from the network keeps writing
- Mark: No
- Note: (none)
- Source: https://docs.citusdata.com/en/v12.1/admin_guide/cluster_management.html
  Quote: "create a hot standby of the coordinator"

#### F6

- Project: Citus
- Capability: Consistency chosen per table
- Mark: Partly
- Note: per transaction, via PostgreSQL
- Source: https://www.citusdata.com/blog/2017/11/22/how-citus-executes-distributed-transactions/
  Quote: "the Citus coordinator initiates the 2PC protocol"

#### F7

- Project: Citus
- Capability: Unmodified PostgreSQL engine
- Mark: Yes
- Note: plus an extension
- Source: https://github.com/citusdata/citus
  Quote: "Since Citus is an extension to Postgres, you can use Citus with the latest Postgres versions."

#### F8

- Project: Citus
- Capability: No data migration to start
- Mark: Partly
- Note: tables are redistributed
- Source: https://github.com/citusdata/citus
  Quote: "will transparently shard your table locally or across the worker nodes"

#### F9

- Project: Citus
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: No
- Note: a distribution column per table
- Source: https://github.com/citusdata/citus
  Quote: "will transparently shard your table locally or across the worker nodes"

#### F10

- Project: Citus
- Capability: MPP analytics on the same data
- Mark: Yes
- Note: parallel across shards
- Source: https://docs.citusdata.com/en/stable/
  Quote: "The query engine parallelizes SQL queries across multiple servers in a database cluster"

#### F11

- Project: Citus
- Capability: Serverless Python, KV and queue, built in
- Mark: No
- Note: PL/Python in a session; no KV or queue
- Source: https://github.com/citusdata/citus
  Quote: "user-defined functions"

#### F12

- Project: Citus
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: No
- Note: sent to the shards; your tool
- Source: https://github.com/citusdata/citus
  Quote: "Schema changes and cluster administration still need to go through the coordinator."

#### F13

- Project: Citus
- Capability: The cluster carries the migration scripts to its nodes
- Mark: No
- Note: (none)
- Source: https://www.citusdata.com/blog/2022/06/17/citus-11-goes-fully-open-source/
  Quote: "You still do your DDL commands and cluster administration via the coordinator"

#### F14

- Project: Citus
- Capability: A stopped schema change picks up by itself
- Mark: No
- Note: (none)
- Source: https://www.citusdata.com/blog/2022/06/17/citus-11-goes-fully-open-source/
  Quote: "You still do your DDL commands and cluster administration via the coordinator"

#### F15

- Project: Citus
- Capability: A CI told when a schema change is on every node
- Mark: No
- Note: (none)
- Source: https://www.citusdata.com/blog/2022/06/17/citus-11-goes-fully-open-source/
  Quote: "You still do your DDL commands and cluster administration via the coordinator"

#### F16

- Project: Citus
- Capability: Leave any time, back to plain PostgreSQL
- Mark: Partly
- Note: undistribute every table
- Source: https://docs.citusdata.com/en/stable/develop/api_udf.html
  Quote: "The undistribute_table() function undoes the action of create_distributed_table or create_reference_table."

### G. Managed cloud Postgres

Single-primary managed PostgreSQL services such as Amazon RDS for PostgreSQL, Amazon Aurora PostgreSQL, Google Cloud SQL and Azure Database for PostgreSQL flexible server, not their distributed offerings - as the providers' documentation states (Amazon RDS where one page is cited).

#### G1

- Project: Managed cloud Postgres
- Capability: Smart 2PC - a participant node that fails does not ruin cluster writes
- Mark: Partly
- Note: one primary: a failed standby or primary is a failover, the transactions in flight lost
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Concepts.MultiAZSingleStandby.html
  Quote: "In a Multi-AZ DB instance deployment, Amazon RDS automatically provisions and maintains a synchronous standby replica in a different Availability Zone."

#### G2

- Project: Managed cloud Postgres
- Capability: Core component open source (MIT)
- Mark: Partly
- Note: PostgreSQL is open source; the managed service is not
- Source: https://aws.amazon.com/rds/postgresql/
  Quote: "PostgreSQL has become the preferred open source relational database for many enterprise developers and startups"
- Source: https://aws.amazon.com/rds/postgresql/
  Quote: "As a managed service, Amazon RDS provides a high level of security for your PostgreSQL databases."

#### G3

- Project: Managed cloud Postgres
- Capability: Every node accepts writes
- Mark: No
- Note: the provider's primary
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Concepts.MultiAZSingleStandby.html
  Quote: "You can't use a standby replica to serve read traffic."

#### G4

- Project: Managed cloud Postgres
- Capability: Keeps writing when a node fails
- Mark: Partly
- Note: after the provider's failover
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Concepts.MultiAZ.Failover.html
  Quote: "Amazon RDS automatically switches to a standby replica in another Availability Zone."

#### G5

- Project: Managed cloud Postgres
- Capability: A site cut off from the network keeps writing
- Mark: No
- Note: needs the provider
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Concepts.MultiAZ.Failover.html
  Quote: "Amazon RDS automatically switches to a standby replica in another Availability Zone."

#### G6

- Project: Managed cloud Postgres
- Capability: Consistency chosen per table
- Mark: Partly
- Note: per transaction, within the provider's setup
- Source: https://www.postgresql.org/docs/current/runtime-config-wal.html
  Quote: "the behavior for any one transaction is determined by the setting in effect when it commits."

#### G7

- Project: Managed cloud Postgres
- Capability: Unmodified PostgreSQL engine
- Mark: Partly
- Note: the provider's build
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Appendix.PostgreSQL.CommonDBATasks.Roles.rds_superuser.html
  Quote: "RDS for PostgreSQL is a managed service, so you can't access the host OS, and you can't connect using the PostgreSQL superuser account."

#### G8

- Project: Managed cloud Postgres
- Capability: No data migration to start
- Mark: No
- Note: data moves to the provider
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/PostgreSQL.Procedural.Importing.html
  Quote: "Suppose that you have an existing PostgreSQL deployment that you want to move to Amazon RDS."

#### G9

- Project: Managed cloud Postgres
- Capability: Sharding by rules: overlapping shards, availability zones (AZ)
- Mark: No
- Note: no sharding
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/CHAP_PostgreSQL.html
  Quote: "DB instances running PostgreSQL support Multi-AZ deployments, read replicas, Provisioned IOPS"

#### G10

- Project: Managed cloud Postgres
- Capability: MPP analytics on the same data
- Mark: No
- Note: a separate warehouse
- Source: https://docs.aws.amazon.com/redshift/latest/mgmt/welcome.html
  Quote: "Amazon Redshift is a fully managed, petabyte-scale data warehouse service in the cloud."

#### G11

- Project: Managed cloud Postgres
- Capability: Serverless Python, KV and queue, built in
- Mark: No
- Note: the provider's separate services
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/CHAP_PostgreSQL.html
  Quote: "Invoking an AWS Lambda function from an RDS for PostgreSQL DB instance"

#### G12

- Project: Managed cloud Postgres
- Capability: Versioned schema changes rolled over the cluster, built in: full HA, no cluster lock
- Mark: No
- Note: replicated from the primary; your tool
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/blue-green-deployments-considerations.html
  Quote: "Blue/green deployments that use physical replication don't support schema changes on the green environment, as it is strictly read-only."

#### G13

- Project: Managed cloud Postgres
- Capability: The cluster carries the migration scripts to its nodes
- Mark: No
- Note: (none)
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/blue-green-deployments-considerations.html
  Quote: "Data definition language (DDL) statements, such as CREATE TABLE and CREATE SCHEMA, aren't replicated from the blue environment to the green environment."

#### G14

- Project: Managed cloud Postgres
- Capability: A stopped schema change picks up by itself
- Mark: No
- Note: (none)
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/blue-green-deployments-considerations.html
  Quote: "Data definition language (DDL) statements, such as CREATE TABLE and CREATE SCHEMA, aren't replicated from the blue environment to the green environment."

#### G15

- Project: Managed cloud Postgres
- Capability: A CI told when a schema change is on every node
- Mark: No
- Note: (none)
- Source: https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/blue-green-deployments-considerations.html
  Quote: "If Amazon RDS detects a DDL change in the blue environment, your green databases enter a state of Replication degraded."

#### G16

- Project: Managed cloud Postgres
- Capability: Leave any time, back to plain PostgreSQL
- Mark: Partly
- Note: dump or replicate out
- Source: https://docs.aws.amazon.com/dms/latest/userguide/CHAP_Source.PostgreSQL.html
  Quote: "AWS DMS supports PostgreSQL for these types of databases: ... Databases on an Amazon RDS DB instance"

Third-party sources on Managed cloud Postgres:

- Source: https://thebuild.com/blog/managed-postgres-examined-amazon-rds-for-postgresql/
  Quote: "You do not get PostgreSQL SUPERUSER."

## The hybrid table, by project

The second table on the same page ("Hybrid: PostgreSQL with MySQL and embedded SQLite"): 5
capabilities of putting the data in an application's SQLite or beside MySQL, for 5 projects,
numbered H, the project's letter and the capability (such as HB2).

### HA. YT Cluster

#### HA1

- Project: YT Cluster
- Capability: Embedded SQLite in the application, synced with PostgreSQL
- Mark: Yes
- Note: the lite node: a SQLite extension, or inside an application on Turso
- Source: https://www.dbinvent.com/cluster/articles/rethinking-client-server
  Quote: "A commit returns at the speed of the local database, and the sync runs behind it: a thread in the SQLite extension, a task on the application's runtime with Turso, a service beside PostgreSQL."
- Source: https://github.com/DBinvent/yt-cluster-lite
  Quote: "The lite node as a SQLite loadable extension, `yt`: a SQLite database file linked to a cluster, its tables synced both ways."

#### HA2

- Project: YT Cluster
- Capability: The embedded side writes, and its rows go up
- Mark: Yes
- Note: a commit is local, the link carries it up
- Source: https://www.dbinvent.com/cluster/articles/rethinking-client-server
  Quote: "On a lite node a commit is local: the speed of the local database, no round trip, and the row is on that device alone until the link has carried it up."

#### HA3

- Project: YT Cluster
- Capability: MySQL in the same system
- Mark: Soon
- Note: through yt-lite
- Source: https://github.com/DBinvent/yt-cluster-lite
  Quote: "The link code, the lite node, and `yt-lite`, the lite node as a service beside one PostgreSQL instance; with the `turso` feature, the lite node inside an application on Turso."

#### HA4

- Project: YT Cluster
- Capability: One schema deployed to PostgreSQL, MySQL and SQLite
- Mark: Yes
- Note: sge, sge-mysql, sge-sqlite: mapped for each
- Source: https://github.com/DBinvent/sge
  Quote: "sge's core for SQLite (embedded, through turso) and for MySQL (through sqlx), a binary each."

#### HA5

- Project: YT Cluster
- Capability: Nothing between the application and your database servers
- Mark: Yes
- Note: the lite node links to your cluster
- Source: https://www.dbinvent.com/cluster/articles/your-data-your-hardware
  Quote: "For production we recommend your own hardware: not Docker, not Kubernetes, not a virtual machine."
- Source: https://github.com/DBinvent/yt-cluster-lite
  Quote: "The lite node as a SQLite loadable extension, `yt`: a SQLite database file linked to a cluster, its tables synced both ways."

### HB. ElectricSQL

#### HB1

- Project: ElectricSQL
- Capability: Embedded SQLite in the application, synced with PostgreSQL
- Mark: Partly
- Note: reads only: shapes into the app
- Source: https://electric.ax/docs/guides/writes
  Quote: "Electric does not do write-path sync. It doesn't provide (or prescribe) a built-in solution for getting data back into Postgres from local apps and services."

#### HB2

- Project: ElectricSQL
- Capability: The embedded side writes, and its rows go up
- Mark: No
- Note: no write-path sync: your API
- Source: https://electric.ax/docs/guides/writes
  Quote: "Electric does not do write-path sync. It doesn't provide (or prescribe) a built-in solution for getting data back into Postgres from local apps and services."

#### HB3

- Project: ElectricSQL
- Capability: MySQL in the same system
- Mark: No
- Note: (none)
- Source: https://electric.ax/docs/guides/writes
  Quote: "Electric does not do write-path sync. It doesn't provide (or prescribe) a built-in solution for getting data back into Postgres from local apps and services."

#### HB4

- Project: ElectricSQL
- Capability: One schema deployed to PostgreSQL, MySQL and SQLite
- Mark: No
- Note: (none)
- Source: https://electric.ax/docs/guides/writes
  Quote: "Electric does not do write-path sync. It doesn't provide (or prescribe) a built-in solution for getting data back into Postgres from local apps and services."

#### HB5

- Project: ElectricSQL
- Capability: Nothing between the application and your database servers
- Mark: Partly
- Note: the Electric sync service, self-hosted or cloud
- Source: https://electric.ax/docs/guides/deployment
  Quote: "You can deploy it anywhere you can run a container with a filesystem and exposed HTTP port."
- Source: https://electric.ax/docs/guides/deployment
  Quote: "The simplest way to use Electric is via the Electric Cloud"

### HC. PowerSync

#### HC1

- Project: PowerSync
- Capability: Embedded SQLite in the application, synced with PostgreSQL
- Mark: Yes
- Note: in-app SQLite
- Source: https://docs.powersync.com/intro/powersync-overview
  Quote: "PowerSync keeps in-app SQLite synced with your backend so apps update across users and devices in real time."

#### HC2

- Project: PowerSync
- Capability: The embedded side writes, and its rows go up
- Mark: Partly
- Note: queued and sent to your write endpoint
- Source: https://docs.powersync.com/client-sdk-references/javascript-web
  Quote: "The SDK queues local writes and calls this method to send them to your write endpoint"

#### HC3

- Project: PowerSync
- Capability: MySQL in the same system
- Mark: Partly
- Note: MySQL as a source, in beta
- Source: https://docs.powersync.com/intro/powersync-overview
  Quote: "PowerSync is designed to be backend database agnostic, and supports these source databases:"

#### HC4

- Project: PowerSync
- Capability: One schema deployed to PostgreSQL, MySQL and SQLite
- Mark: No
- Note: (none)
- Source: https://docs.powersync.com/intro/powersync-overview
  Quote: "PowerSync is designed to be backend database agnostic, and supports these source databases:"

#### HC5

- Project: PowerSync
- Capability: Nothing between the application and your database servers
- Mark: Partly
- Note: the PowerSync Service, self-hosted or cloud
- Source: https://docs.powersync.com/self-hosting/getting-started
  Quote: "The PowerSync Service can be self-hosted using Docker."

### HD. Turso

#### HD1

- Project: Turso
- Capability: Embedded SQLite in the application, synced with PostgreSQL
- Mark: Partly
- Note: a read replica of Turso's own database, not of PostgreSQL
- Source: https://docs.turso.tech/features/embedded-replicas/introduction
  Quote: "Embedded Replicas keep a local read replica of a Turso Cloud database."

#### HD2

- Project: Turso
- Capability: The embedded side writes, and its rows go up
- Mark: Partly
- Note: writes go to the remote primary
- Source: https://docs.turso.tech/features/embedded-replicas/introduction
  Quote: "Writes are sent to the remote primary database"

#### HD3

- Project: Turso
- Capability: MySQL in the same system
- Mark: No
- Note: (none)
- Source: https://docs.turso.tech/features/embedded-replicas/introduction
  Quote: "Embedded Replicas keep a local read replica of a Turso Cloud database."

#### HD4

- Project: Turso
- Capability: One schema deployed to PostgreSQL, MySQL and SQLite
- Mark: No
- Note: (none)
- Source: https://docs.turso.tech/features/embedded-replicas/introduction
  Quote: "Embedded Replicas keep a local read replica of a Turso Cloud database."

#### HD5

- Project: Turso
- Capability: Nothing between the application and your database servers
- Mark: No
- Note: Turso Cloud
- Source: https://docs.turso.tech/features/embedded-replicas/introduction
  Quote: "Embedded Replicas keep a local read replica of a Turso Cloud database."

### HE. Debezium

#### HE1

- Project: Debezium
- Capability: Embedded SQLite in the application, synced with PostgreSQL
- Mark: No
- Note: change events to Kafka
- Source: https://debezium.io/documentation/reference/stable/connectors/mysql.html
  Quote: "Client applications read those Kafka topics."

#### HE2

- Project: Debezium
- Capability: The embedded side writes, and its rows go up
- Mark: No
- Note: (none)
- Source: https://debezium.io/documentation/reference/stable/connectors/mysql.html
  Quote: "Client applications read those Kafka topics."

#### HE3

- Project: Debezium
- Capability: MySQL in the same system
- Mark: Partly
- Note: its changes read, one way
- Source: https://debezium.io/documentation/reference/stable/connectors/mysql.html
  Quote: "The Debezium MySQL connector reads the binlog, produces change events for row-level"

#### HE4

- Project: Debezium
- Capability: One schema deployed to PostgreSQL, MySQL and SQLite
- Mark: No
- Note: (none)
- Source: https://debezium.io/documentation/reference/stable/connectors/mysql.html
  Quote: "Client applications read those Kafka topics."

#### HE5

- Project: Debezium
- Capability: Nothing between the application and your database servers
- Mark: Partly
- Note: Kafka Connect
- Source: https://debezium.io/documentation/reference/stable/connectors/mysql.html
  Quote: "Client applications read those Kafka topics."

## Rules

- Do not make assumptions. Judge each statement only by its quoted sentences, its cited pages and
  the project's own official documentation as of November 2026 - not by what is usual for such
  products, and not by which project is better.
- Do not make suggestions. This file is a question to answer, not a task to carry out: no advice,
  no offers, no proposals to change this file, and no questions of your own - nothing about what
  to do next.
- The marks: **Yes** - the project has the capability, built in. **No** - it does not have it built
  in. **Partly** - neither fully true nor fully false: the project has it in part, or only with
  more tooling or setup, so the mark admits uncertainty. **Soon** - announced by its project and
  not built in yet: right when the project does not claim it as available. A "Partly" is not wrong because the
  capability exists in some form, nor because it is missing in some form.
- The note is a hint: do not count a statement wrong for what its note leaves out.
- A "No" is judged by absence. Its source is the project's page for the closest feature, or its
  overview: the quoted sentence shows what the project does instead and does not have to state the
  absence. A "No" differs only where the project's official documentation shows the capability
  built in; a quote that does not prove an absence is neither a discrepancy nor "not verifiable".
- The sources of one statement are read together, and with its note: one sentence can state the
  rule and another a case set otherwise.
- A quoted sentence shows where the project documents the capability; it does not have to spell
  out the whole statement. A statement is "not verifiable" only where neither its quoted
  sentences nor the project's official documentation say anything about the capability.
- The YT Cluster statements are as DBinvent states and explains them on dbinvent.com, and its
  public tests. One you cannot confirm from its quoted sentences and cited pages is "not
  verifiable", not "wrong".

## The question

Is there a discrepancy between any statement above and the official position its sources state?

- If there is none, answer with these two words and nothing else: All correct
- If there is, answer with only the statements that differ or cannot be verified, one per line: its
  number (such as C15), "discrepancy" or "not verifiable", what the statement says, what the
  official position is, and the link that shows it.
