interface ImportMeta {
  glob: (pattern: string, options?: { eager?: boolean; query?: string; import?: string }) => Record<string, string>;
}
