-- Migration: create telegram_users table
CREATE TABLE IF NOT EXISTS telegram_users (
    id              BIGINT PRIMARY KEY,         -- Telegram user ID
    username        TEXT,
    first_name      TEXT NOT NULL,
    last_name       TEXT,
    language_code   TEXT,
    is_premium      BOOLEAN NOT NULL DEFAULT FALSE,
    role            TEXT NOT NULL DEFAULT 'USER',
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    blocked_bot     BOOLEAN NOT NULL DEFAULT FALSE,
    start_count     INTEGER NOT NULL DEFAULT 1,
    is_in_group     BOOLEAN NOT NULL DEFAULT FALSE,
    is_in_channel   BOOLEAN NOT NULL DEFAULT FALSE,
    last_active_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
