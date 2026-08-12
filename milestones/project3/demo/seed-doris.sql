-- Apache Doris fixture for the demo.
--
-- The same eight rows as seed-mysql.sql. The DDL is not the same and is not
-- meant to be: Doris needs a distribution clause and a replication setting,
-- and has no PRIMARY KEY in the MySQL sense. That difference is part of the
-- point — two genuinely different engines, one identical query.
CREATE DATABASE IF NOT EXISTS shop;
USE shop;

DROP TABLE IF EXISTS orders;
CREATE TABLE orders (
  id     INT         NOT NULL,
  region VARCHAR(16) NOT NULL,
  amount INT         NOT NULL
)
DISTRIBUTED BY HASH(id) BUCKETS 1
PROPERTIES ("replication_num" = "1");

INSERT INTO orders (id, region, amount) VALUES
  (1, 'EU',   100),
  (2, 'US',   200),
  (3, 'APAC', 150),
  (4, 'EU',   250),
  (5, 'US',   300),
  (6, 'APAC', 120),
  (7, 'EU',    80),
  (8, 'US',   175);
