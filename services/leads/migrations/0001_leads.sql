-- Pricing requests from the "Contact for pricing" form on logicaffeine.com/pricing.
-- No IP address is kept: country is Cloudflare's two-letter code for where the request came from.
-- notified_at is when the sales inbox was emailed about the lead; it stays NULL when a lead for the
-- same address was emailed about in the last 10 minutes, or when the email failed.
CREATE TABLE leads (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    email        TEXT NOT NULL,
    phone        TEXT,
    phone_digits TEXT,
    source       TEXT,
    country      TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    notified_at  TEXT,
    status       TEXT NOT NULL DEFAULT 'new',
    CHECK ((phone IS NULL) = (phone_digits IS NULL))
);

CREATE INDEX idx_leads_email_notified ON leads (email COLLATE NOCASE, notified_at);
CREATE INDEX idx_leads_created ON leads (created_at);
