// Holds the raw observations of the fields with a custom merge until finalize.
//
// Kept in memory by default. Given a directory, observations are appended to one file per
// field and entity partition instead, so memory use no longer grows with the number of
// submissions: finalize reads back one partition at a time (see groups).

import { appendFileSync, mkdirSync, readFileSync, rmSync } from "fs";
import { join } from "path";
import type { Observation } from "./observation";

const DISK_PARTITIONS = 64;
/** Fields read back as a whole in the order they were added (see stream), so never split. */
const UNPARTITIONED_FIELDS = new Set(["turnInPoi"]);
/** Buffered output is written once it gets this large. */
const FLUSH_BYTES = 32 * 1024 * 1024;

export class ObservationStore {
  private readonly partitions: number;
  private readonly memory = new Map<string, Observation<unknown>[][]>();
  private readonly buffers = new Map<string, string[]>();
  private bufferedBytes = 0;

  constructor(private readonly dir?: string) {
    this.partitions = dir === undefined ? 1 : DISK_PARTITIONS;
    if (dir !== undefined) {
      rmSync(dir, { recursive: true, force: true });
      mkdirSync(dir, { recursive: true });
    }
  }

  add(field: string, observations: Observation<unknown>[]): void {
    for (const observation of observations) {
      const partition = UNPARTITIONED_FIELDS.has(field) ? 0 : Math.abs(observation.entityId) % this.partitions;
      if (this.dir === undefined) {
        this.memoryPartitions(field)[partition].push(observation);
        continue;
      }
      const path = this.path(field, partition);
      let buffer = this.buffers.get(path);
      if (!buffer) {
        buffer = [];
        this.buffers.set(path, buffer);
      }
      const line = JSON.stringify(observation);
      buffer.push(line);
      this.bufferedBytes += line.length + 1;
    }
    if (this.bufferedBytes >= FLUSH_BYTES) this.flush();
  }

  /**
   * Every observation of `fields` grouped by entity, one partition at a time. Per entity, the
   * observations of each field come in the order they were added, the fields one after another.
   */
  *groups<T>(fields: string[]): Generator<Map<number, Observation<T>[]>> {
    this.flush();
    for (let partition = 0; partition < this.partitions; partition++) {
      const byEntity = new Map<number, Observation<T>[]>();
      for (const field of fields) {
        for (const observation of this.read<T>(field, partition)) {
          const list = byEntity.get(observation.entityId);
          if (list) list.push(observation);
          else byEntity.set(observation.entityId, [observation]);
        }
      }
      yield byEntity;
    }
  }

  /** Every observation of `field` in chunks, in the order they were added. */
  *stream<T>(field: string): Generator<Observation<T>[]> {
    this.flush();
    for (let partition = 0; partition < this.partitions; partition++) {
      const chunk = this.read<T>(field, partition);
      if (chunk.length > 0) yield chunk;
    }
  }

  clear(field: string): void {
    this.memory.delete(field);
    if (this.dir === undefined) return;
    for (let partition = 0; partition < this.partitions; partition++) {
      const path = this.path(field, partition);
      const lines = this.buffers.get(path);
      if (lines) this.bufferedBytes -= lines.reduce((sum, line) => sum + line.length + 1, 0);
      this.buffers.delete(path);
      rmSync(path, { force: true });
    }
  }

  /** Every observation of `field`, for small fields and tests. */
  all<T>(field: string): Observation<T>[] {
    this.flush();
    const out: Observation<T>[] = [];
    for (let partition = 0; partition < this.partitions; partition++) out.push(...this.read<T>(field, partition));
    return out;
  }

  /** Removes the files on disk. */
  dispose(): void {
    if (this.dir !== undefined) rmSync(this.dir, { recursive: true, force: true });
  }

  private flush(): void {
    for (const [path, lines] of this.buffers) {
      if (lines.length > 0) appendFileSync(path, lines.join("\n") + "\n");
    }
    this.buffers.clear();
    this.bufferedBytes = 0;
  }

  private read<T>(field: string, partition: number): Observation<T>[] {
    if (this.dir === undefined) return (this.memory.get(field)?.[partition] ?? []) as Observation<T>[];
    let text: string;
    try {
      text = readFileSync(this.path(field, partition), "utf8");
    } catch {
      return [];
    }
    const out: Observation<T>[] = [];
    let start = 0;
    while (start < text.length) {
      const end = text.indexOf("\n", start);
      out.push(JSON.parse(text.slice(start, end)));
      start = end + 1;
    }
    return out;
  }

  private memoryPartitions(field: string): Observation<unknown>[][] {
    let partitions = this.memory.get(field);
    if (!partitions) {
      partitions = Array.from({ length: this.partitions }, () => []);
      this.memory.set(field, partitions);
    }
    return partitions;
  }

  private path(field: string, partition: number): string {
    return join(this.dir!, `${field}.${partition}.ndjson`);
  }
}
