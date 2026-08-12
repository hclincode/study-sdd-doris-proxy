-- MySQL fixture for the demo.
--
-- Eight orders across three regions; three of them are EU, which is what the
-- proxy's row filter selects. Identical rows to seed-doris.sql — only the DDL
-- differs, because the engines genuinely differ.
CREATE DATABASE IF NOT EXISTS shop;
USE shop;

DROP TABLE IF EXISTS orders;
CREATE TABLE orders (
  id     INT         NOT NULL PRIMARY KEY,
  region VARCHAR(16) NOT NULL,
  amount INT         NOT NULL
);

INSERT INTO orders (id, region, amount) VALUES
  (1, 'EU',   100),
  (2, 'US',   200),
  (3, 'APAC', 150),
  (4, 'EU',   250),
  (5, 'US',   300),
  (6, 'APAC', 120),
  (7, 'EU',    80),
  (8, 'US',   175);
