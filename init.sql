-- =============================================================================
-- DataForge Database Initialization Script
-- =============================================================================
-- Sets up core tables, indexes, and initial data for DataForge backend services.
-- =============================================================================

CREATE TABLE IF NOT EXISTS file_processing_jobs (
    job_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    filename VARCHAR(255) NOT NULL,
    file_format VARCHAR(50) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'PENDING',
    total_rows BIGINT DEFAULT 0,
    processed_rows BIGINT DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS parsed_file_metadata (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID REFERENCES file_processing_jobs(job_id) ON DELETE CASCADE,
    headers JSONB NOT NULL,
    column_count INT NOT NULL,
    file_size_bytes BIGINT NOT NULL,
    checksum_md5 VARCHAR(32),
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS transformation_audit_logs (
    log_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID REFERENCES file_processing_jobs(job_id) ON DELETE CASCADE,
    transformation_type VARCHAR(100) NOT NULL,
    rule_config JSONB NOT NULL,
    rows_affected BIGINT DEFAULT 0,
    executed_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS user_formulas (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    expression TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Indexes for high performance queries
CREATE INDEX IF NOT EXISTS idx_jobs_status ON file_processing_jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_created_at ON file_processing_jobs(created_at);
CREATE INDEX IF NOT EXISTS idx_audit_job_id ON transformation_audit_logs(job_id);

-- Seed initial default formulas
INSERT INTO user_formulas (name, expression, description) VALUES
('Total Sales', 'SUM(A1:A100)', 'Calculates total sum of sales revenue'),
('Average Rating', 'AVERAGE(B1:B50)', 'Calculates average rating score'),
('PII Mask Rule', 'MASK_EMAIL(C1)', 'Masks customer emails for privacy compliance')
ON CONFLICT DO NOTHING;
