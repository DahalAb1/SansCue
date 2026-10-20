#!/bin/sh
# Sourced by the official PostgreSQL entrypoint only for a fresh data volume.
# psql quotes identifiers/literals; passwords never appear in shell arguments.
psql -X --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" --set ON_ERROR_STOP=1 <<'SQL'
\getenv app_user SANSCUE_APP_USER
\getenv app_db SANSCUE_APP_DB
\getenv app_password SANSCUE_APP_PASSWORD
CREATE ROLE :"app_user" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION PASSWORD :'app_password';
CREATE DATABASE :"app_db" OWNER :"app_user";
SQL
