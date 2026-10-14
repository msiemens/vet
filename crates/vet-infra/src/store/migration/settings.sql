CREATE TABLE "backend_setting" ( "key" varchar NOT NULL PRIMARY KEY, "value" integer NOT NULL );

CREATE TABLE "preference" ( "scope" varchar NOT NULL, "key" varchar NOT NULL, "value" varchar NOT NULL, CONSTRAINT "pk-preference" PRIMARY KEY ("scope", "key") );
