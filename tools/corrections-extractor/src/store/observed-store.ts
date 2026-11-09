// Observations of every processed submission, kept between runs so a run only has to observe
// the submissions added since the last one.
//
// Layout of the store directory:
//   manifest.json                     format and observer version and, per live submission
//                                     key, its batch, load errors and counts
//   batches/<n>.sessions.parquet      one row per session copy, including the copies not
//                                     observed because a newer one was used (`duplicate`)
//   batches/<n>.observations.parquet  one row per observation of the observed copies
//
// Rows are never rewritten. A submission that disappears from the source is dropped from the
// manifest only, and only the batches and submissions the manifest lists are read. When the
// observer code changes (see observer-version.ts) the whole store is dropped.
//
// Replay hands the sessions over in the order a full run folds them: submissions newest first,
// their sessions in export order, and of each session (by key) only the first copy met.

import { DuckDBInstance, type DuckDBAppender, type DuckDBConnection } from "@duckdb/node-api";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "fs";
import { join } from "path";
import type { StagedSession } from "../extract";
import type { StagedLookupSession } from "../extract/lookups";
import type { Confidence, Observation } from "../extract/observation";
import type { ObservedSubmission } from "../observe-submission";

const FORMAT_VERSION = 2;

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
  formatVersion: number;
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

const KINDS = 3;
/** Submissions replayed per query. */
const REPLAY_SLICE = 2000;
const KIND_SCALAR = 0;
const KIND_OBSERVATION = 1;
const KIND_LOOKUP = 2;

function sqlString(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

function appendNullable(appender: DuckDBAppender, value: string | number | null | undefined): void {
  if (value === null || value === undefined) appender.appendNull();
  else if (typeof value === "number") appender.appendDouble(value);
  else appender.appendVarchar(value);
}

/** What an observation holds beyond its columns (e.g. the spawn observers' `token`), as JSON; null when nothing. */
function extraOf(observation: Observation<unknown>): string | null {
  const { entityId: _entityId, value: _value, confidence: _confidence, provenance, ...rest } = observation;
  const { session: _session, t: _t, ...provenanceRest } = provenance;
  const hasRest = Object.keys(rest).length > 0;
  const hasProvenanceRest = Object.keys(provenanceRest).length > 0;
  if (!hasRest && !hasProvenanceRest) return null;
  return JSON.stringify({ ...(hasRest ? { o: rest } : {}), ...(hasProvenanceRest ? { p: provenanceRest } : {}) });
}

export class ObservedStore {
  private constructor(
    private readonly dir: string,
    private readonly manifest: Manifest,
    private readonly instance: DuckDBInstance,
    private readonly connection: DuckDBConnection,
  ) {}

  /** Opens the store in `dir`, dropping its contents when they were made by another observer version. */
  static async open(dir: string, observerVersion: string): Promise<ObservedStore> {
    mkdirSync(dir, { recursive: true });
    let manifest = readManifest(dir);
    if (!manifest || manifest.observerVersion !== observerVersion || manifest.formatVersion !== FORMAT_VERSION) {
      rmSync(join(dir, "batches"), { recursive: true, force: true });
      manifest = { formatVersion: FORMAT_VERSION, observerVersion, nextBatch: 0, submissions: {} };
      writeManifest(dir, manifest);
    }
    mkdirSync(join(dir, "batches"), { recursive: true });
    // Left behind by a run that was killed.
    rmSync(join(dir, ".tmp"), { recursive: true, force: true });
    const instance = await DuckDBInstance.create(":memory:", { temp_directory: join(dir, ".tmp"), memory_limit: "4GB", max_temp_directory_size: "20GB" });
    try {
      return new ObservedStore(dir, manifest, instance, await instance.connect());
    } catch (e) {
      instance.closeSync();
      throw e;
    }
  }

  close(): void {
    this.connection.closeSync();
    this.instance.closeSync();
    rmSync(join(this.dir, ".tmp"), { recursive: true, force: true });
  }

  keys(): Set<string> {
    return new Set(Object.keys(this.manifest.submissions));
  }

  entries(): [string, ManifestEntry][] {
    return Object.entries(this.manifest.submissions);
  }

  /** Drops submissions from the store. */
  forget(keys: Iterable<string>): void {
    for (const key of keys) delete this.manifest.submissions[key];
    writeManifest(this.dir, this.manifest);
  }

  /**
   * The other stored submissions holding unobserved copies of sessions that `keys` observed:
   * without `keys`, those copies are needed, so these submissions must be observed again.
   */
  async holdingCopiesOf(keys: string[]): Promise<string[]> {
    const removed = new Set(keys);
    const entries = this.entries();
    if (entries.length === 0 || keys.length === 0) return [];
    await this.loadSubmissions("removed", entries.filter(([key]) => removed.has(key)));
    await this.loadSubmissions("kept", entries.filter(([key]) => !removed.has(key)));
    const reader = await this.connection.runAndReadAll(`
      SELECT DISTINCT copy.submission
      FROM ${this.sessionsOf("kept")} copy
      WHERE copy.duplicate AND copy.session_key IN (
        SELECT used.session_key FROM ${this.sessionsOf("removed")} used WHERE NOT used.duplicate AND used.session_key IS NOT NULL
      )`);
    return reader.getRowObjectsJS().map((row) => row.submission as string);
  }

  /** Stores the observations of `submissions` as one batch. */
  async add(submissions: StoredSubmission[]): Promise<void> {
    if (submissions.length === 0) return;
    const batch = this.manifest.nextBatch;
    const c = this.connection;
    await c.run(`CREATE OR REPLACE TEMP TABLE sessions (submission VARCHAR, batch INTEGER, idx INTEGER, session_key VARCHAR, duplicate BOOLEAN, contributor VARCHAR, session_time DOUBLE, has_extract BOOLEAN, has_lookup BOOLEAN, error VARCHAR)`);
    await c.run(`CREATE OR REPLACE TEMP TABLE observations (submission VARCHAR, batch INTEGER, idx INTEGER, kind TINYINT, pos SMALLINT, field VARCHAR, seq INTEGER, entity DOUBLE, confidence VARCHAR, session_name VARCHAR, t DOUBLE, value VARCHAR, extra VARCHAR)`);
    const sessions = await c.createAppender("sessions");
    try {
      const observations = await c.createAppender("observations");
      try {
        const appendFields = (key: string, idx: number, kind: number, fields: [string, Observation<unknown>[]][]) => {
          fields.forEach(([field, list], pos) => {
            list.forEach((observation, seq) => {
              observations.appendVarchar(key);
              observations.appendInteger(batch);
              observations.appendInteger(idx);
              observations.appendTinyInt(kind);
              observations.appendSmallInt(pos);
              observations.appendVarchar(field);
              observations.appendInteger(seq);
              observations.appendDouble(observation.entityId);
              observations.appendVarchar(observation.confidence);
              appendNullable(observations, observation.provenance.session);
              appendNullable(observations, observation.provenance.t);
              observations.appendVarchar(JSON.stringify(observation.value) ?? "null");
              appendNullable(observations, extraOf(observation));
              observations.endRow();
            });
          });
        };

        for (const { key, submission } of submissions) {
          submission.sessions.forEach((session, idx) => {
            const staged = session.extract ?? session.lookup;
            sessions.appendVarchar(key);
            sessions.appendInteger(batch);
            sessions.appendInteger(idx);
            appendNullable(sessions, session.key);
            sessions.appendBoolean(session.duplicate === true);
            appendNullable(sessions, staged?.contributorId);
            appendNullable(sessions, staged?.sessionTime);
            sessions.appendBoolean(session.extract !== undefined);
            sessions.appendBoolean(session.lookup !== undefined);
            appendNullable(sessions, session.error);
            sessions.endRow();
            appendFields(key, idx, KIND_SCALAR, session.extract?.scalars ?? []);
            appendFields(key, idx, KIND_OBSERVATION, session.extract?.observations ?? []);
            appendFields(key, idx, KIND_LOOKUP, (session.lookup?.observations ?? []) as [string, Observation<unknown>[]][]);
          });
        }
      } finally {
        observations.closeSync();
      }
    } finally {
      sessions.closeSync();
    }

    // Both files are written under temporary names first, so a crash never leaves half a batch.
    const sessionsFile = this.batchFile(batch, "sessions");
    const observationsFile = this.batchFile(batch, "observations");
    await c.run(`COPY sessions TO ${sqlString(`${sessionsFile}.tmp`)} (FORMAT parquet, COMPRESSION zstd)`);
    await c.run(`COPY (SELECT * FROM observations ORDER BY submission, idx, kind, pos, seq) TO ${sqlString(`${observationsFile}.tmp`)} (FORMAT parquet, COMPRESSION zstd)`);
    await c.run("DROP TABLE sessions");
    await c.run("DROP TABLE observations");
    renameSync(`${sessionsFile}.tmp`, sessionsFile);
    renameSync(`${observationsFile}.tmp`, observationsFile);

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
   * Returns how many observed session copies were left out because a newer copy was used.
   */
  async replay(
    onSession: (session: ReplayedSession) => void,
    onProgress?: (submissions: number, total: number) => void,
    slice = REPLAY_SLICE,
  ): Promise<{ duplicateCopies: number }> {
    const entries = this.entries();
    if (entries.length === 0) return { duplicateCopies: 0 };
    const c = this.connection;
    await this.loadSubmissions("live", entries);

    await c.run(`
      CREATE OR REPLACE TEMP TABLE winners AS
      SELECT * EXCLUDE (copy) FROM (
        SELECT s.*, row_number() OVER (PARTITION BY s.session_key ORDER BY s.rank, s.idx) AS copy
        FROM ${this.sessionsOf("live")} s
        WHERE NOT s.duplicate
      ) WHERE session_key IS NULL OR copy = 1`);
    const counts = await c.runAndReadAll(`
      SELECT (SELECT count(*) FROM ${this.sessionsOf("live")} s WHERE NOT s.duplicate) - (SELECT count(*) FROM winners) AS duplicates`);
    const duplicateCopies = Number(counts.getRowObjectsJS()[0].duplicates);

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

    // A slice of submissions at a time: sorting every observation at once spills tens of GB.
    // Observation files are sorted by submission, so each slice only reads its own row groups.
    const sorted = entries.map(([key]) => key).sort().reverse();
    for (let start = 0; start < sorted.length; start += slice) {
      const newest = sorted[start];
      const oldest = sorted[Math.min(start + slice, sorted.length) - 1];
      const range = `BETWEEN ${sqlString(oldest)} AND ${sqlString(newest)}`;
      const result = await c.stream(`
        SELECT w.submission, w.idx, w.session_key, w.contributor, w.session_time, w.has_extract, w.has_lookup, w.error,
               o.kind, o.field, o.entity, o.confidence, o.session_name, o.t, o.value, o.extra
        FROM (SELECT * FROM winners WHERE submission ${range}) w
        LEFT JOIN (SELECT * FROM read_parquet(${this.files("observations")}) WHERE submission ${range}) o
          ON o.submission = w.submission AND o.batch = w.batch AND o.idx = w.idx
        ORDER BY w.rank, w.idx, o.kind, o.pos, o.seq`);
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
            current = { submission, idx, session, fields: Array.from({ length: KINDS }, () => new Map()) };
          }
          if (row.kind === null) continue;
          const byField = current.fields[row.kind as number];
          const field = row.field as string;
          let list = byField.get(field);
          if (!list) {
            list = [];
            byField.set(field, list);
          }
          const extra = row.extra === null ? {} : (JSON.parse(row.extra as string) as { o?: object; p?: object });
          list.push({
            ...extra.o,
            entityId: row.entity as number,
            value: JSON.parse(row.value as string),
            confidence: row.confidence as Confidence,
            provenance: { session: row.session_name as string, t: row.t as number, ...extra.p },
          });
        }
      }
      onProgress?.(Math.min(start + slice, sorted.length), sorted.length);
    }
    flush();
    return { duplicateCopies };
  }

  private batchFile(batch: number, kind: "sessions" | "observations"): string {
    return join(this.dir, "batches", `${batch}.${kind}.parquet`);
  }

  /** Loads `entries` into table `name` (submission, batch, rank), rank 0 being the newest submission. */
  private async loadSubmissions(name: string, entries: [string, ManifestEntry][]): Promise<void> {
    const sorted = [...entries].sort(([a], [b]) => (a < b ? 1 : a > b ? -1 : 0));
    await this.connection.run(`CREATE OR REPLACE TEMP TABLE ${name} (submission VARCHAR, batch INTEGER, rank INTEGER)`);
    const appender = await this.connection.createAppender(name);
    try {
      sorted.forEach(([key, entry], rank) => {
        appender.appendVarchar(key);
        appender.appendInteger(entry.batch);
        appender.appendInteger(rank);
        appender.endRow();
      });
    } finally {
      appender.closeSync();
    }
  }

  /** The parquet files of the batches the manifest points at. */
  private files(kind: "sessions" | "observations"): string {
    const batches = [...new Set(this.entries().map(([, entry]) => entry.batch))].sort((a, b) => a - b);
    return `[${batches.map((batch) => sqlString(this.batchFile(batch, kind))).join(", ")}]`;
  }

  /** The session rows of the submissions in table `name`, with their rank. */
  private sessionsOf(name: string): string {
    return `(SELECT s.*, ${name}.rank FROM read_parquet(${this.files("sessions")}) s JOIN ${name} USING (submission, batch))`;
  }
}

function readManifest(dir: string): Manifest | undefined {
  const path = join(dir, "manifest.json");
  if (!existsSync(path)) return undefined;
  try {
    return JSON.parse(readFileSync(path, "utf8")) as Manifest;
  } catch {
    // A torn write: start over rather than fail every run.
    return undefined;
  }
}

function writeManifest(dir: string, manifest: Manifest): void {
  const path = join(dir, "manifest.json");
  writeFileSync(`${path}.tmp`, JSON.stringify(manifest), { flush: true });
  renameSync(`${path}.tmp`, path);
}
