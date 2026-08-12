-- The one query every case runs.
--
-- 02-direct.sh and 03-proxied.sh both read this file, so "all four cases run
-- identical SQL" is something you can check rather than something you're told.
-- The database is selected on the command line, not here, because connection
-- parameters differ per engine while the query does not.
SELECT * FROM orders ORDER BY id;
