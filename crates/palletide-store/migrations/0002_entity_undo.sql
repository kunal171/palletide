-- Durable before-images for opt-in application tables. Keep the original
-- bookkeeping migration unchanged so existing Palletide databases can upgrade.
CREATE TABLE palletide_entity_table (
    relation OID PRIMARY KEY,
    primary_key TEXT[] NOT NULL,
    since_height BIGINT NOT NULL
);
CREATE TABLE palletide_entity_undo (
    id BIGSERIAL PRIMARY KEY,
    block_height BIGINT NOT NULL,
    relation OID NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('INSERT', 'UPDATE', 'DELETE')),
    old_row JSONB,
    new_row JSONB
);
CREATE INDEX palletide_entity_undo_height_idx ON palletide_entity_undo (block_height);
CREATE TABLE palletide_undo_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    pruned_through BIGINT NOT NULL DEFAULT -1
);
INSERT INTO palletide_undo_state DEFAULT VALUES;

CREATE FUNCTION palletide_capture_entity() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE height TEXT;
BEGIN
    IF current_setting('palletide.restoring', TRUE) = 'on' THEN
        RETURN NULL;
    END IF;
    height := current_setting('palletide.block_height', TRUE);
    IF height IS NULL OR height = '' THEN
        RAISE EXCEPTION 'tracked table % requires a Palletide block transaction', TG_RELID::regclass;
    END IF;
    IF TG_OP = 'UPDATE' AND to_jsonb(OLD) = to_jsonb(NEW) THEN
        RETURN NULL;
    END IF;
    INSERT INTO public.palletide_entity_undo (block_height, relation, operation, old_row, new_row)
    VALUES (height::BIGINT, TG_RELID, TG_OP,
            CASE WHEN TG_OP <> 'INSERT' THEN to_jsonb(OLD) END,
            CASE WHEN TG_OP <> 'DELETE' THEN to_jsonb(NEW) END);
    RETURN NULL;
END;
$$;

CREATE FUNCTION palletide_reject_truncate() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'TRUNCATE is not supported on tracked Palletide tables; use DELETE';
END;
$$;

CREATE FUNCTION palletide_track_table(target REGCLASS) RETURNS VOID
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE keys TEXT[];
BEGIN
    PERFORM pg_advisory_xact_lock(72548371044);
    IF EXISTS (SELECT 1 FROM public.palletide_entity_table WHERE relation = target::OID) THEN
        RETURN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_class WHERE oid = target AND relkind = 'r')
       OR EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = target OR inhparent = target) THEN
        RAISE EXCEPTION 'Palletide tracking requires an ordinary, non-inherited table';
    END IF;
    IF (SELECT relnamespace FROM pg_class WHERE oid = target) = 'public'::regnamespace
       AND (SELECT relname FROM pg_class WHERE oid = target) LIKE 'palletide\_%' ESCAPE '\' THEN
        RAISE EXCEPTION 'cannot track framework bookkeeping tables';
    END IF;
    IF EXISTS (SELECT 1 FROM pg_constraint
               WHERE contype = 'f' AND (conrelid = target OR confrelid = target)
                 AND (confupdtype IN ('c', 'n', 'd') OR confdeltype IN ('c', 'n', 'd'))) THEN
        RAISE EXCEPTION 'tracked tables cannot use cascading or SET NULL/DEFAULT foreign keys';
    END IF;
    EXECUTE format('LOCK TABLE %s IN ACCESS EXCLUSIVE MODE', target);
    SELECT array_agg(a.attname ORDER BY k.ordinality) INTO keys
    FROM pg_index i
    CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ordinality)
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
    WHERE i.indrelid = target AND i.indisprimary AND k.ordinality <= i.indnkeyatts;
    IF keys IS NULL THEN
        RAISE EXCEPTION 'tracked table % must have a primary key', target;
    END IF;
    INSERT INTO public.palletide_entity_table (relation, primary_key, since_height)
    SELECT target, keys, coalesce(max(height), -1) FROM public.palletide_block;
    EXECUTE format('CREATE TRIGGER palletide_capture AFTER INSERT OR UPDATE OR DELETE ON %s FOR EACH ROW EXECUTE FUNCTION public.palletide_capture_entity()', target);
    EXECUTE format('CREATE TRIGGER palletide_no_truncate BEFORE TRUNCATE ON %s FOR EACH STATEMENT EXECUTE FUNCTION public.palletide_reject_truncate()', target);
END;
$$;

CREATE FUNCTION palletide_restore_entities(fork_height BIGINT) RETURNS VOID
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE entry RECORD; keys TEXT[]; columns TEXT; values_sql TEXT; predicate TEXT; affected BIGINT;
BEGIN
    PERFORM pg_advisory_xact_lock(72548371044);
    IF fork_height < (SELECT pruned_through FROM public.palletide_undo_state) THEN
        RAISE EXCEPTION 'entity undo history has been pruned above fork height %', fork_height;
    END IF;
    IF EXISTS (SELECT 1 FROM public.palletide_entity_table WHERE since_height > fork_height) THEN
        RAISE EXCEPTION 'entity tracking started after fork height %; reindex from a clean database', fork_height;
    END IF;
    PERFORM set_config('palletide.restoring', 'on', TRUE);
    SET CONSTRAINTS ALL DEFERRED;
    FOR entry IN SELECT * FROM public.palletide_entity_undo
                 WHERE block_height > fork_height ORDER BY id DESC LOOP
        SELECT primary_key INTO STRICT keys FROM public.palletide_entity_table WHERE relation = entry.relation;
        SELECT string_agg(format('%I', attname), ', ' ORDER BY attnum),
               string_agg(format('r.%I', attname), ', ' ORDER BY attnum)
        INTO columns, values_sql FROM pg_attribute
        WHERE attrelid = entry.relation AND attnum > 0 AND NOT attisdropped AND attgenerated = ''
          AND (entry.operation <> 'UPDATE' OR attidentity <> 'a');
        SELECT string_agg(format('t.%1$I IS NOT DISTINCT FROM r.%1$I', key), ' AND ')
        INTO predicate FROM unnest(keys) AS key;
        IF entry.operation = 'INSERT' THEN
            EXECUTE format('DELETE FROM %s t USING jsonb_populate_record(NULL::%s, $1) r WHERE %s',
                           entry.relation::regclass, entry.relation::regclass, predicate)
            USING entry.new_row;
        ELSIF entry.operation = 'DELETE' THEN
            EXECUTE format('INSERT INTO %s (%s) OVERRIDING SYSTEM VALUE SELECT %s FROM jsonb_populate_record(NULL::%s, $1) r',
                           entry.relation::regclass, columns, values_sql, entry.relation::regclass)
            USING entry.old_row;
        ELSE
            EXECUTE format('UPDATE %s t SET (%s) = (SELECT %s FROM jsonb_populate_record(NULL::%s, $1) r) FROM jsonb_populate_record(NULL::%s, $2) r WHERE %s',
                           entry.relation::regclass, columns, values_sql, entry.relation::regclass,
                           entry.relation::regclass, predicate)
            USING entry.old_row, entry.new_row;
        END IF;
        GET DIAGNOSTICS affected = ROW_COUNT;
        IF affected <> 1 THEN
            RAISE EXCEPTION 'cannot restore entity undo entry %: expected one affected row, got %', entry.id, affected;
        END IF;
    END LOOP;
    DELETE FROM public.palletide_entity_undo WHERE block_height > fork_height;
    PERFORM set_config('palletide.restoring', 'off', TRUE);
END;
$$;
