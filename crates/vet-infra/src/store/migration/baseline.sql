CREATE TABLE "snapshot" ( "id" varbinary_blob NOT NULL PRIMARY KEY, "created" integer NOT NULL );

CREATE TABLE "snapshot_path" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "snapshot" varbinary_blob NOT NULL, "flag" integer NOT NULL, "path" varbinary_blob NOT NULL, FOREIGN KEY ("snapshot") REFERENCES "snapshot" ("id") ON DELETE CASCADE );

CREATE TABLE "tree_node" ( "id" varbinary_blob NOT NULL PRIMARY KEY );

CREATE TABLE "tree_child" ( "node" varbinary_blob NOT NULL, "name" varbinary_blob NOT NULL, "kind" integer NOT NULL, "dir" varbinary_blob, "blob" varbinary_blob, "mode" integer, "entry_kind" integer, "unfiltered" boolean, CONSTRAINT "pk-tree_child" PRIMARY KEY ("node", "name"), FOREIGN KEY ("node") REFERENCES "tree_node" ("id") ON DELETE CASCADE );

CREATE TABLE "changeset" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "repo" varchar NOT NULL, "kind" integer NOT NULL, "key" varchar NOT NULL, "lens" integer, "title" varchar, "draft" boolean NOT NULL DEFAULT FALSE, "fork" boolean NOT NULL DEFAULT FALSE, "trusted_head" varchar, "target_branch" varchar, "pinned_base" varchar, "vote" varchar, "vote_posting_since" integer, "vote_error" varchar );

CREATE UNIQUE INDEX "idx-changeset-source" ON "changeset" ("repo", "kind", "key");

CREATE TABLE "version" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "changeset" integer NOT NULL, "n" integer NOT NULL, "base" varbinary_blob NOT NULL, "base_commit" varchar NOT NULL, "target" varbinary_blob NOT NULL, "tip" varchar NOT NULL, "at" integer NOT NULL, "origin" integer NOT NULL, FOREIGN KEY ("changeset") REFERENCES "changeset" ("id") ON DELETE CASCADE );

CREATE UNIQUE INDEX "idx-version-number" ON "version" ("changeset", "n");

CREATE TABLE "version_file" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "version" integer NOT NULL, "path" varbinary_blob NOT NULL, "added" integer NOT NULL, "removed" integer NOT NULL, FOREIGN KEY ("version") REFERENCES "version" ("id") ON DELETE CASCADE );

CREATE TABLE "review" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "changeset" integer NOT NULL, "base_kind" integer NOT NULL, "base_value" varchar NOT NULL, "last_reviewed" integer, "opened_at" integer NOT NULL, "closed_at" integer, "complete" boolean NOT NULL, "summary_total" integer, "summary_seen" integer, "summary_old_base" integer, FOREIGN KEY ("changeset") REFERENCES "changeset" ("id") ON DELETE CASCADE );

CREATE TABLE "project_filter" ( "repo" varchar NOT NULL PRIMARY KEY, "hide_binary" boolean NOT NULL );

CREATE TABLE "project_filter_item" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "repo" varchar NOT NULL, "kind" integer NOT NULL, "value" varbinary_blob NOT NULL, FOREIGN KEY ("repo") REFERENCES "project_filter" ("repo") ON DELETE CASCADE );

CREATE TABLE "project_trust" ( "repo" varchar NOT NULL PRIMARY KEY );

CREATE TABLE "anchor" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "changeset" integer NOT NULL, "side" integer NOT NULL, "snapshot" varbinary_blob NOT NULL, "file" varbinary_blob NOT NULL, "start" integer NOT NULL, "end" integer NOT NULL, "raw" varbinary_blob NOT NULL, "norm" varbinary_blob NOT NULL, "context" varbinary_blob NOT NULL, FOREIGN KEY ("changeset") REFERENCES "changeset" ("id") ON DELETE CASCADE );

CREATE TABLE "span" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "review" integer NOT NULL, "state" integer NOT NULL, "base_anchor" integer, "target_anchor" integer, "origin_commit" varchar, FOREIGN KEY ("review") REFERENCES "review" ("id") ON DELETE CASCADE );

CREATE TABLE "ack" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "review" integer NOT NULL, "change" integer NOT NULL, "before_path" varbinary_blob, "before_blob" varbinary_blob, "before_mode" integer, "before_kind" integer, "before_unfiltered" boolean, "after_path" varbinary_blob, "after_blob" varbinary_blob, "after_mode" integer, "after_kind" integer, "after_unfiltered" boolean, FOREIGN KEY ("review") REFERENCES "review" ("id") ON DELETE CASCADE );

CREATE TABLE "thread" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "changeset" integer NOT NULL, "review" integer, "kind" integer NOT NULL, "file_side" integer, "file_path" varbinary_blob, "anchor" integer, "status" integer NOT NULL, "on_platform" boolean NOT NULL, "remote_id" varchar, "resolvable" boolean NOT NULL, "resolved_by" varchar, "status_pending" boolean NOT NULL, FOREIGN KEY ("changeset") REFERENCES "changeset" ("id") ON DELETE CASCADE, FOREIGN KEY ("review") REFERENCES "review" ("id") ON DELETE SET NULL );

CREATE TABLE "message" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "thread" integer NOT NULL, "author" varchar NOT NULL, "body" varchar NOT NULL, "at" integer NOT NULL, "mine" boolean NOT NULL, "avatar" varchar, "edited_at" integer, "remote_id" varchar, "synced" varchar, "conflict_body" varchar, "conflict_at" integer, "posting_since" integer, "post_error" varchar, FOREIGN KEY ("thread") REFERENCES "thread" ("id") ON DELETE CASCADE );

CREATE TABLE "server_path" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "language" varchar NOT NULL UNIQUE, "path" varchar NOT NULL );

CREATE TABLE "recent_project" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "path" varchar NOT NULL UNIQUE, "opened_at" integer NOT NULL );
