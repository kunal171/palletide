-- Support partition roots, cascading foreign keys and transactional schema checkpoints.
ALTER TABLE palletide_entity_undo ADD COLUMN cascaded BOOLEAN NOT NULL DEFAULT FALSE;

CREATE OR REPLACE FUNCTION palletide_capture_entity() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE height TEXT; root_relation OID;
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
    root_relation := coalesce(TG_ARGV[0]::OID, TG_RELID);
    INSERT INTO public.palletide_entity_undo (block_height, relation, operation, old_row, new_row, cascaded)
    VALUES (height::BIGINT, root_relation, TG_OP,
            CASE WHEN TG_OP <> 'INSERT' THEN to_jsonb(OLD) END,
            CASE WHEN TG_OP <> 'DELETE' THEN to_jsonb(NEW) END, pg_trigger_depth() > 1);
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION palletide_track_table(target REGCLASS) RETURNS VOID
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE keys TEXT[];
BEGIN
    PERFORM pg_advisory_xact_lock(72548371044);
    IF NOT EXISTS (SELECT 1 FROM pg_class WHERE oid = target AND relkind IN ('r','p') AND relpersistence = 'p') THEN
        RAISE EXCEPTION 'tracking requires a persistent ordinary or partitioned table';
    END IF;
    IF EXISTS (SELECT 1 FROM pg_class WHERE oid = target AND relispartition)
       OR (EXISTS (SELECT 1 FROM pg_class WHERE oid = target AND relkind = 'r')
           AND EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = target OR inhparent = target)) THEN
        RAISE EXCEPTION 'register the partition root; ordinary inheritance is unsupported';
    END IF;
    IF (SELECT relnamespace FROM pg_class WHERE oid = target) = 'public'::regnamespace
       AND (SELECT relname FROM pg_class WHERE oid = target) LIKE 'palletide\_%' ESCAPE '\' THEN
        RAISE EXCEPTION 'cannot track framework bookkeeping tables';
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
    IF EXISTS (SELECT 1 FROM public.palletide_entity_table t
               WHERE t.relation = target AND t.primary_key <> keys)
       AND EXISTS (SELECT 1 FROM public.palletide_entity_undo WHERE relation = target) THEN
        RAISE EXCEPTION 'checkpoint entity history before changing primary keys';
    END IF;
    INSERT INTO public.palletide_entity_table (relation, primary_key, since_height)
    SELECT target, keys, coalesce(max(height), -1) FROM public.palletide_block
    ON CONFLICT (relation) DO UPDATE SET primary_key = EXCLUDED.primary_key;
    EXECUTE format('DROP TRIGGER IF EXISTS palletide_capture ON %s', target);
    EXECUTE format('DROP TRIGGER IF EXISTS palletide_no_truncate ON %s', target);
    EXECUTE format('CREATE TRIGGER palletide_capture AFTER INSERT OR UPDATE OR DELETE ON %s FOR EACH ROW EXECUTE FUNCTION public.palletide_capture_entity(%L)', target, target::OID::TEXT);
    EXECUTE format('CREATE TRIGGER palletide_no_truncate BEFORE TRUNCATE ON %s FOR EACH STATEMENT EXECUTE FUNCTION public.palletide_reject_truncate()', target);
END;
$$;

CREATE OR REPLACE FUNCTION palletide_restore_entities(fork_height BIGINT) RETURNS VOID
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE entry RECORD; keys TEXT[]; columns TEXT; values_sql TEXT; predicate TEXT; affected BIGINT;
        constraint_row RECORD; immediate_constraints JSONB;
BEGIN
    PERFORM pg_advisory_xact_lock(72548371044);
    IF fork_height < (SELECT pruned_through FROM public.palletide_undo_state) THEN
        RAISE EXCEPTION 'entity undo history has been pruned above fork height %', fork_height;
    END IF;
    IF EXISTS (SELECT 1 FROM public.palletide_entity_table WHERE since_height > fork_height) THEN
        RAISE EXCEPTION 'entity tracking started after fork height %; reindex from a clean database', fork_height;
    END IF;
    IF fork_height > 0
       AND EXISTS (SELECT 1 FROM public.palletide_block WHERE height > fork_height)
       AND NOT EXISTS (SELECT 1 FROM public.palletide_block WHERE height = fork_height) THEN
        RAISE EXCEPTION 'block history unavailable at fork height %', fork_height;
    END IF;
    -- FK cascade AFTER triggers can journal children after parents. Temporarily
    -- defer immediate FK checks so their inverse order can reconstruct the full
    -- graph. Restore constraint definitions before committing; errors roll back
    -- both the DDL and all row changes. The tracking user owns these tables.
    SELECT coalesce(jsonb_agg(jsonb_build_object('table_name', conrelid::regclass::TEXT, 'name', conname)), '[]'::JSONB)
    INTO immediate_constraints FROM pg_constraint
    WHERE contype = 'f' AND NOT condeferrable AND conparentid = 0
      AND (conrelid IN (SELECT relation FROM public.palletide_entity_table)
           OR confrelid IN (SELECT relation FROM public.palletide_entity_table));
    FOR constraint_row IN SELECT * FROM jsonb_to_recordset(immediate_constraints) AS c(table_name TEXT, name TEXT) LOOP
        EXECUTE format('ALTER TABLE %s ALTER CONSTRAINT %I DEFERRABLE INITIALLY IMMEDIATE', constraint_row.table_name, constraint_row.name);
    END LOOP;
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
        -- A parent inverse may already have cascaded the child's inverse.
        -- Only FK-triggered entries can be skipped, and only if their complete
        -- before-image already exists (not merely their key).
        IF entry.cascaded AND entry.operation <> 'INSERT' THEN
            EXECUTE format('SELECT count(*) FROM %s t WHERE to_jsonb(t) = $1', entry.relation::regclass)
            INTO affected USING entry.old_row;
            IF affected = 1 THEN CONTINUE; END IF;
        END IF;
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
        IF affected = 0 AND entry.cascaded AND entry.operation = 'INSERT' THEN
            CONTINUE;
        END IF;
        IF affected <> 1 THEN
            RAISE EXCEPTION 'cannot restore entity undo entry %: expected one affected row, got %', entry.id, affected;
        END IF;
    END LOOP;
    SET CONSTRAINTS ALL IMMEDIATE;
    FOR constraint_row IN SELECT * FROM jsonb_to_recordset(immediate_constraints) AS c(table_name TEXT, name TEXT) LOOP
        EXECUTE format('ALTER TABLE %s ALTER CONSTRAINT %I NOT DEFERRABLE', constraint_row.table_name, constraint_row.name);
    END LOOP;
    DELETE FROM public.palletide_entity_undo WHERE block_height > fork_height;
    PERFORM set_config('palletide.restoring', 'off', TRUE);
END;
$$;

CREATE FUNCTION palletide_checkpoint_entities() RETURNS VOID
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE boundary BIGINT;
BEGIN
    PERFORM pg_advisory_xact_lock(72548371044);
    SELECT coalesce(max(height), -1) INTO boundary FROM public.palletide_block;
    DELETE FROM public.palletide_entity_undo;
    UPDATE public.palletide_entity_table SET since_height = greatest(since_height, boundary);
    UPDATE public.palletide_undo_state SET pruned_through = greatest(pruned_through, boundary);
END;
$$;
