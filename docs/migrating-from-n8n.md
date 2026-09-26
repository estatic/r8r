# Migrating from n8n

`r8r migrate-from-n8n` copies an n8n 1.x or 2.x instance into r8r. It reads
the n8n database (SQLite or PostgreSQL) and never writes to it, so n8n keeps
working on it and going back is just starting n8n again.

## Steps

1. Stop n8n (or at least its workers and webhook processes), so nothing
   changes during the import.
2. Use the **same `N8N_ENCRYPTION_KEY`** as the n8n instance. Credentials are
   copied still encrypted and only this key opens them. The import stops
   before writing anything if the key does not fit.
3. Give r8r **its own database**. r8r and n8n both default to
   `~/.n8n/database.sqlite`, and r8r refuses to open an n8n database. Point
   r8r elsewhere with `DB_SQLITE_DATABASE=/path/r8r.sqlite`, or with
   `DB_TYPE=postgresdb` and another `DB_POSTGRESDB_SCHEMA`.
4. Run the import:

   ```sh
   # n8n on SQLite
   N8N_ENCRYPTION_KEY=... DB_SQLITE_DATABASE=/data/r8r.sqlite \
     r8r migrate-from-n8n --db=sqlite:/home/node/.n8n/database.sqlite

   # n8n on PostgreSQL, r8r on PostgreSQL too
   N8N_ENCRYPTION_KEY=... DB_TYPE=postgresdb DB_POSTGRESDB_SCHEMA=r8r \
     r8r migrate-from-n8n --db=postgres://n8n:secret@db:5432/n8n --schema=public
   ```

   Options: `--schema` (the n8n tables' PostgreSQL schema, default `public`),
   `--table-prefix` (n8n's `DB_TABLE_PREFIX`), `--skip-executions`.
5. Start r8r (`r8r start`) with the same settings.

Running the import again updates what changed and keeps executions that are
already there, so it can be repeated right before switching over.

## What comes across

| n8n | In r8r |
| --- | --- |
| Users | Same ids and roles; passwords (bcrypt) work unchanged; pending invitations stay pending |
| Public API keys | The keys n8n issued keep working, with their scopes |
| Workflows | Same ids, tags and settings; active (published) workflows are active again. In n8n 2.x an active workflow's *published* version is imported; unpublished draft changes are reported and left out |
| Credentials | Same ids, still encrypted with `N8N_ENCRYPTION_KEY` |
| Projects | Team projects with their members and roles; personal projects become ownership by their user |
| Variables | Keys and values |
| Executions | History with run data (n8n's "flatted" data is unpacked). Waiting executions resume: the resume URLs n8n handed out (`/webhook-waiting/<id>?signature=...`) keep working |

Not imported: archived workflows, n8n's own settings (SSO, LDAP, license,
event destinations), folders, insights and evaluations. Production webhook
URLs stay the same (`/webhook/<path>`), so callers need no changes.
