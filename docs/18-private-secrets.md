# Local-only secrets and citations (0.6.5)

The **secret** category holds developer-specific filenames, paths and other private
text separately from public knowledge. Entries live only in `.akr/secrets/`, excluded
from public record discovery, source-graph hashes, locks, SQLite indexes and generated
views. They are immutable local JSON entries with an opaque identity, not public AKR
records with a new lifecycle kind. They cannot serve as normative authority, acceptance
evidence or graph relations. A public clone needs no secret files to build or validate.

## Create and cite

Put the exact UTF-8 value in a private file first. Keep that input file outside tracked
or published material too. The command reads the file without echoing its path or value:

```sh
akr secret put --from <private-input-file>
```

It returns a token such as `{{secret:0123456789abcdef0123456789abcdef}}`. The 32 lowercase
hex digits are randomly seeded, independent of the value, and carry no private name,
path, digest of the value or retrieval URL. Use the complete token inside quoted text
slots, titles or claims, for example:

```akr
intent "Read the developer configuration from {{secret:0123456789abcdef0123456789abcdef}}."
```

That source text is safe to commit. Normal retrieval, context and generated views show:

```text
Read the developer configuration from [*].
```

Canonical retrieval (`akr get <record> --detail canonical`) retains opaque tokens for
round-trip editing and later local resolution. It never contains the private values.
Secret citations are inline text, separate from `@key` references: do not put them in
typed Git commits, relation targets, source locators, command flags, scope or watch
globs. Resolve a private path locally before using it operationally.

## Local resolution

```sh
akr secret get '{{secret:0123456789abcdef0123456789abcdef}}'
# [*]
akr secret get '{{secret:0123456789abcdef0123456789abcdef}}' --reveal
# the local value, if present; otherwise [*]
akr secret resolve --from <document-with-citations> --reveal
```

Normal rendering **always** redacts, including on developer machines. Only explicit
`--reveal` reads private data. `resolve` substitutes inline tokens in one pass, preserves
the rest of the document and never changes the input file. Missing entries become `[*]`.
Private output is for local use; do not save it to tracked files or publish it. Entries
are immutable: to change a value, create a new entry and revise public citations through
the normal record workflow. There is no automatic expiry or scratch pruning of secrets.

The matching MCP tools are `knowledge.secret_put` (`from`), `knowledge.secret_get`
(`ref`, optional `reveal`) and `knowledge.secret_resolve` (`from`, optional `reveal`).
`reveal` defaults to false. **A revealed MCP response goes to the caller and its model
provider.** Use the local CLI for credentials that must stay on the developer's machine.
Agents should store and cite private information without copying it into public prose,
evidence commands, registered sources or handoffs. The MCP handshake, installed agent
section and `akr init` guidance teach this workflow.

## Sharing and publication boundary

New workspaces ignore `.akr/secrets/`. The first private write also creates an internal
ignore file, protecting existing workspaces before setup is rerun. AKR check, build and
commit preparation refuse any private-store path in the Git index, including force-added
files (`AKR-C046`). Public source/import and generic input-file workflows refuse inputs
from that directory. These protections do not rewrite Git history or police external
archive/upload tools. Exclude `.akr/secrets/` from whole-directory backups or archives
intended for publication; publish the Git tree or generated views instead.

The public ledger has no backing values, reversible encoding, lookup endpoint or
value-derived hashes. A reader who receives only that ledger cannot retrieve private
values through AKR, even knowing every opaque id. Other developers need a separate,
trusted transfer of the private store; AKR provides no network sharing service. Share
only with developers entitled to those values.

Private storage is plaintext, protected by the developer's OS account and filesystem.
On Unix AKR creates directories with mode 0700 and entry files with mode 0600; on Windows
they inherit the workspace ACL. Storage symlinks are refused. This is a publication
boundary, not encryption or a substitute for an OS credential vault. Existing values
already copied into tracked files or Git history must be removed separately; exposed
credentials need rotation. AKR cannot infer which arbitrary prose is private: use the
secret category before authoring public records.
