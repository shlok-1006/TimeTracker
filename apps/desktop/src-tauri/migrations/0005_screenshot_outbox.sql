-- 0005_screenshot_outbox.sql — screenshots waiting to reach storage + the API.
--
-- Capture used to ask the server for an upload URL first, so with the server
-- unreachable (a VPN that blocks us, no Wi-Fi) no screenshot was taken at all
-- and the day reached the AI review empty. Now every capture is written here
-- first (Rule 1: local first) and an uploader drains the queue whenever the
-- server answers, sending the ORIGINAL capture time.
--
-- The JPEG itself is a file `<app data>/screenshot-outbox/<id>.bin`, encrypted
-- with a device key kept in the OS keychain. A row is deleted together with
-- its file once the API has the metadata.

CREATE TABLE IF NOT EXISTS screenshot_outbox (
    id              TEXT PRIMARY KEY NOT NULL,   -- uuid; also the file name
    user_id         TEXT NOT NULL,               -- uploaded only for this user
    taken_at        TEXT NOT NULL,               -- RFC3339 UTC capture time
    captured_status TEXT NOT NULL,               -- working | meeting
    -- Set once the bytes are in storage, so a retry after a failed metadata
    -- POST re-sends only the metadata instead of uploading the image twice.
    storage_key     TEXT,
    -- Times the server REFUSED this row (a 4xx). Network failures don't count:
    -- a week offline must not age a screenshot out by retries alone.
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT
);

CREATE INDEX IF NOT EXISTS idx_screenshot_outbox_user
    ON screenshot_outbox (user_id, attempts, taken_at);
