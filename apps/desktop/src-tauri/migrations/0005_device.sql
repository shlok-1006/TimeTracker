-- 0005_device.sql — this install's device id.
--
-- A random id minted once, the first time it is needed, and sent with every presence
-- heartbeat. Someone signed in on two machines then has one live status per machine on the
-- server, and their badge shows the best of them instead of flipping to whichever machine
-- reported last. It names the install, not the person, and carries nothing about the machine.
-- At most one row.

CREATE TABLE IF NOT EXISTS device (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    device_id TEXT    NOT NULL
);
