ALTER TABLE domain_events ADD COLUMN lifecycle TEXT NOT NULL DEFAULT 'active'
    CHECK (lifecycle IN ('active', 'superseded', 'invalidated'));

ALTER TABLE objects ADD COLUMN lifecycle TEXT NOT NULL DEFAULT 'active'
    CHECK (lifecycle IN ('active', 'superseded', 'invalidated'));
