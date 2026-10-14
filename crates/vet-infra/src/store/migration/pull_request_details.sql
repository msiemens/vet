CREATE TABLE "pull_request" ( "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "host" varchar NOT NULL, "path" varchar NOT NULL, "number" integer NOT NULL, "head" varchar NOT NULL, "updated_at" integer NOT NULL, "synced_at" integer NOT NULL, "detail" varchar NOT NULL );

CREATE UNIQUE INDEX "idx-pull_request-key" ON "pull_request" ("host", "path", "number");
