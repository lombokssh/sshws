-- Migration: message log
CREATE TABLE IF NOT EXISTS messages (
    id                  BIGSERIAL PRIMARY KEY,
    message_id          INTEGER NOT NULL,
    chat_id             BIGINT NOT NULL,
    chat_title          TEXT,                      -- group/channel name
    chat_type           TEXT NOT NULL,             -- private, group, supergroup, channel
    user_id             BIGINT,                    -- NULL for channel posts
    text                TEXT,                      -- text or caption
    message_type        TEXT NOT NULL DEFAULT 'text',
    reply_to_message_id INTEGER,                   -- ID of the message this replies to
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS messages_chat_id_idx ON messages (chat_id);
CREATE INDEX IF NOT EXISTS messages_user_id_idx ON messages (user_id);
