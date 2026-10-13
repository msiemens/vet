# Bundled schemas

The JSON schemas the YAML and JSON language servers check files against when the project is not
trusted and the network stays closed (`crate::schemas`). The set is fixed; a change to a file or to the
set changes the fingerprint of the bundle, and with it the identity of every server that uses it (I2).

Every file is the upstream file with its white space removed. Retrieved on 2026-10-13.

| File | Upstream | Licence |
|---|---|---|
| `github-workflow.schema.json` | https://json.schemastore.org/github-workflow.json | Apache-2.0 (SchemaStore) |
| `azure-pipelines.schema.json` | https://raw.githubusercontent.com/microsoft/azure-pipelines-vscode/main/service-schema.json | MIT (Microsoft) |
| `compose-spec.schema.json` | https://raw.githubusercontent.com/compose-spec/compose-spec/main/schema/compose-spec.json | Apache-2.0 (Compose Specification) |
| `package-json.schema.json` | https://json.schemastore.org/package.json | Apache-2.0 (SchemaStore) |
| `tsconfig.schema.json` | https://json.schemastore.org/tsconfig.json | Apache-2.0 (SchemaStore) |
| `kubernetes-all.schema.json` | https://raw.githubusercontent.com/yannh/kubernetes-json-schema/master/v1.34.0-standalone-strict/all.json | Apache-2.0 (yannh/kubernetes-json-schema) |
| `kubernetes-definitions.schema.json` | the same directory's `_definitions.json` | as above |

`package-json.schema.json` also has the `$ref`s to other schemas of the schema store (eslintrc,
prettierrc, ...) removed: they would be requests for files that are not bundled.

The licences and notices are in `THIRD_PARTY_NOTICES.md`.
