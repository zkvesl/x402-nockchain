CREATE TABLE catalog (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    resource      TEXT NOT NULL,
    type          TEXT NOT NULL,
    x402_version  INTEGER NOT NULL,
    accepts       TEXT NOT NULL,     -- JSON-serialized Vec<PaymentRequirements>
    last_updated  TEXT NOT NULL,     -- ISO 8601
    metadata      TEXT,              -- JSON-serialized DiscoveryInfo echo
    UNIQUE(resource, type)
);
CREATE INDEX catalog_type_idx ON catalog (type);
