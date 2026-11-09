// Observations of every processed submission, kept between runs so a run only has to observe
// the submissions added since the last one.
//
// Layout of the store directory:
//   manifest.json                     observer version and, per live submission key, its batch,
//                                     load errors and counts
//   batches/<n>.sessions.parquet      one row per observed session copy
//   batches/<n>.observations.parquet  one row per observation of those sessions
//
// Rows are never rewritten. A submission that disappears from the source is dropped from the
// manifest only, and replay skips rows of submissions (and batches) the manifest does not list.
// When the observer code changes (see observer-version.ts) the whole store is dropped.
//
// Replay hands the sessions over in the order a full run folds them: submissions newest first,
// their sessions in export order, and of each session (by key) only the first copy met.

import { DuckDBInstance, type DuckDBConnection } from "@duckdb/node-api";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "fs";
import { join } from "path";
import type { StagedSession } from "../extract";
import type { StagedLookupSession } from "../extract/lookups";
import type { Confidence, Observation } from "../extract/observation";
import type { ObservedSubmission } from "../observe-submission";

const MANIFEST_VERSION = 1;

export interface ManifestEntry {
  batch: number;
  submissionId: string;
  exportCount: number;
  droppedExports: string[];
  error?: string;
  /** Session copies not observed because a newer copy was used. */
  duplicates: number;
}

interface Manifest {
  manifestVersion: number;
  observerVersion: string;
  nextBatch: number;
  submissions: Record<string, ManifestEntry>;
}

export interface StoredSubmission {
  /** Source key, see store/source.ts. */
  key: string;
  submission: ObservedSubmission;
}

/** One session as replay hands it over. */
export interface ReplayedSession {
  submissionKey: string;
  /** Set when an observer threw on this session (see ObservedSession.error). */
  error?: string;
  extract?: StagedSession;
  lookup?: StagedLookupSession;
}

const KIND_SCALAR = 0;
const KIND_OBSERVATION = 1;
const KIND_LOOKUP = 2;

function sqlString(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

export class ObservedStore {
  private constructor(
    private readonly dir: string,
    private readonly manifest: Manifest,
    private readonly connection: DuckDBConnection,
  ) {}

  /** Opens the store in `dir`, dropping its contents when they were made by another observer version. */
  static async open(dir: string, observerVersion: string): Promise<ObservedStore> {
    mkdirSync(dir, { recursive: true });
    let manifest = readManifest(dir);
    if (!manifest || manifest.observerVersion !== observerVersion || manifest.manifestVersion !== MANIFEST_VERSION) {
      rmSync(join(dir, "batches"), { recursive: true, force: true });
      manifest = { manifestVersion: MANIFEST_VERSION, observerVersion, nextBatch: 0, submissions: {} };
      writeManifest(dir, manifest);
    }
    mkdirSync(join(dir, "batches"), { recursive: true });
    const instance = await DuckDBInstance.create(":memory:", { temp_directory: join(dir, ".tmp") });
    return new ObservedStore(dir, manifest, await instance.connect());
  }

  close(): void {
    this.connection.closeSync();
    rmSync(join(this.dir, ".tmp"), { recursive: true, force: true });
  }

  keys(): Set<string> {
    return new Set(Object.keys(this.manifest.submissions));
  }

  entries(): [string, ManifestEntry][] {
    return Object.entries(this.manifest.submissions);
  }

  /** Drops submissions that are no longer in the source. */
  forget(keys: Iterable<string>): void {
    for (const key of keys) delete this.manifest.submissions[key];
    writeManifest(this.dir, this.manifest);
  }

  /** Stores the observations of `submissions` as one batch. */
  async add(submissions: StoredSubmission[]): Promise<void> {
    if (submissions.length === 0) return;
    const batch = this.manifest.nextBatch;
    const c = this.connection;
    await c.run(`CREATE OR REPLACE TEMP TABLE sessions (submission VARCHAR, batch INTEGER, idx INTEGER, session_key VARCHAR, contributor VARCHAR, session_time DOUBLE, has_extract BOOLEAN, has_lookup BOOLEAN, error VARCHAR)`);
    await c.run(`CREATE OR REPLACE TEMP TABLE observations (submission VARCHAR, batch INTEGER, idx INTEGER, kind TINYINT, field VARCHAR, seq INTEGER, entity DOUBLE, confidence VARCHAR, session_name VARCHAR, t DOUBLE, value VARCHAR)`);
    const sessions = await c.createAppender("sessions");
    const observations = await c.createAppender("observations");
    const appendObservations = (key: string, idx: number, kind: number, field: string, list: Observation<unknown>[]) => {
      list.forEach((observation, seq) => {
        observations.appendVarchar(key);
        observations.appendInteger(batch);
        observations.appendInteger(idx);
        observations.appendTinyInt(kind);
        observations.appendVarchar(field);
        observations.appendInteger(seq);
        observations.appendDouble(observation.entityId);
        observations.appendVarchar(observation.confidence);
        observations.appendVarchar(observation.provenance.session);
        observations.appendDouble(observation.provenance.t);
        observations.appendVarchar(JSON.stringify(observation.value) ?? "null");
        observations.endRow();
      });
    };
    const nullable = (appender: typeof sessions, value: string | number | null | undefined) => {
      if (value === null || value === undefined) appender.appendNull();
      else if (typeof value === "number") appender.appendDouble(value);
      else appender.appendVarchar(value);
    };

    for (const { key, submission } of submissions) {
      submission.sessions.forEach((session, idx) => {
        if (session.duplicate) return;
        const staged = session.extract ?? session.lookup;
        sessions.appendVarchar(key);
        sessions.appendInteger(batch);
        sessions.appendInteger(idx);
        nullable(sessions, session.key);
        nullable(sessions, staged?.contributorId);
        nullable(sessions, staged?.sessionTime);
        sessions.appendBoolean(session.extract !== undefined);
        sessions.appendBoolean(session.lookup !== undefined);
        nullable(sessions, session.error);
        sessions.endRow();
        for (const [field, list] of session.extract?.scalars ?? []) appendObservations(key, idx, KIND_SCALAR, field, list);
        for (const [field, list] of session.extract?.observations ?? []) appendObservations(key, idx, KIND_OBSERVATION, field, list);
        for (const [field, list] of session.lookup?.observations ?? []) appendObservations(key, idx, KIND_LOOKUP, field, list as Observation<unknown>[]);
      });
    }
    sessions.closeSync();
    observations.closeSync();

    const base = join(this.dir, "batches", String(batch));
    await c.run(`COPY sessions TO ${sqlString(`${base}.sessions.parquet`)} (FORMAT parquet, COMPRESSION zstd)`);
    await c.run(`COPY (SELECT * FROM observations ORDER BY field, entity) TO ${sqlString(`${base}.observations.parquet`)} (FORMAT parquet, COMPRESSION zstd)`);
    await c.run("DROP TABLE sessions");
    await c.run("DROP TABLE observations");

    // Only now that the batch is written: a crash before leaves files no manifest entry points at.
    for (const { key, submission } of submissions) {
      this.manifest.submissions[key] = {
        batch,
        submissionId: submission.submissionId,
        exportCount: submission.exportCount,
        droppedExports: submission.droppedExports,
        ...(submission.error === undefined ? {} : { error: submission.error }),
        duplicates: submission.sessions.filter((session) => session.duplicate).length,
      };
    }
    this.manifest.nextBatch = batch + 1;
    writeManifest(this.dir, this.manifest);
  }

  /**
   * Hands every used session to `onSession`, in fold order (see the top of this file).
   * Returns how many stored session copies were left out because a newer copy was used.
   */
  async replay(onSession: (session: ReplayedSession) => void): Promise<{ duplicateCopies: number }> {
    const c = this.connection;
    await c.run("CREATE OR REPLACE TEMP TABLE live (submission VARCHAR, batch INTEGER)");
    const live = await c.createAppender("live");
    for (const [key, entry] of this.entries()) {
      live.appendVarchar(key);
      live.appendInteger(entry.batch);
      live.endRow();
    }
    live.closeSync();

    const batches = join(this.dir, "batches");
    if (!readdirSync(batches).some((name) => name.endsWith(".sessions.parquet"))) return { duplicateCopies: 0 };
    const sessionFiles = sqlString(join(batches, "*.sessions.parquet"));
    const observationFiles = sqlString(join(batches, "*.observations.parquet"));

    await c.run(`
      CREATE OR REPLACE TEMP TABLE winners AS
      SELECT * EXCLUDE (copy) FROM (
        SELECT s.*, row_number() OVER (PARTITION BY s.session_key ORDER BY s.submission DESC, s.idx) AS copy
        FROM read_parquet(${sessionFiles}) s JOIN live USING (submission, batch)
      ) WHERE session_key IS NULL OR copy = 1`);
    const counts = await c.runAndReadAll(`
      SELECT (SELECT count(*) FROM read_parquet(${sessionFiles}) s JOIN live USING (submission, batch)) - (SELECT count(*) FROM winners) AS duplicates`);
    const duplicateCopies = Number(counts.getRowObjectsJS()[0].duplicates);

    const result = await c.stream(`
      SELECT w.submission, w.idx, w.session_key, w.contributor, w.session_time, w.has_extract, w.has_lookup, w.error,
             o.kind, o.field, o.entity, o.confidence, o.session_name, o.t, o.value
      FROM winners w LEFT JOIN read_parquet(${observationFiles}) o
        ON o.submission = w.submission AND o.batch = w.batch AND o.idx = w.idx
      ORDER BY w.submission DESC, w.idx, o.kind, o.field, o.seq`);

    let current: { submission: string; idx: number; session: ReplayedSession; fields: Map<string, Observation<unknown>[]>[] } | undefined;
    const flush = () => {
      if (!current) return;
      const [scalars, observations, lookups] = current.fields;
      const { extract, lookup } = current.session;
      if (extract) {
        extract.scalars = [...scalars] as StagedSession["scalars"];
        extract.observations = [...observations] as StagedSession["observations"];
      }
      if (lookup) lookup.observations = [...lookups] as StagedLookupSession["observations"];
      onSession(current.session);
    };

    for await (const rows of result.yieldRowObjectJs()) {
      for (const row of rows) {
        const submission = row.submission as string;
        const idx = row.idx as number;
        if (!current || current.submission !== submission || current.idx !== idx) {
          flush();
          const identity = {
            ...(row.session_key === null ? {} : { sessionKey: row.session_key as string }),
            ...(row.contributor === null ? {} : { contributorId: row.contributor as string }),
          };
          const sessionTime = (row.session_time as number | null) ?? null;
          const session: ReplayedSession = { submissionKey: submission };
          if (row.error !== null) session.error = row.error as string;
          if (row.has_extract) session.extract = { ...identity, sessionTime, scalars: [], observations: [] };
          if (row.has_lookup) session.lookup = { ...identity, sessionTime, observations: [] };
          current = { submission, idx, session, fields: [new Map(), new Map(), new Map()] };
        }
        if (row.kind === null) continue;
        const byField = current.fields[row.kind as number];
        const field = row.field as string;
        let list = byField.get(field);
        if (!list) {
          list = [];
          byField.set(field, list);
        }
        list.push({
          entityId: row.entity as number,
          value: JSON.parse(row.value as string),
          confidence: row.confidence as Confidence,
          provenance: { session: row.session_name as string, t: row.t as number },
        });
      }
    }
    flush();
    return { duplicateCopies };
  }
}

function readManifest(dir: string): Manifest | undefined {
  const path = join(dir, "manifest.json");
  if (!existsSync(path)) return undefined;
  return JSON.parse(readFileSync(path, "utf8")) as Manifest;
}

function writeManifest(dir: string, manifest: Manifest): void {
  const path = join(dir, "manifest.json");
  writeFileSync(`${path}.tmp`, JSON.stringify(manifest));
  renameSync(`${path}.tmp`, path);
}
