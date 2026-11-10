import { useId, useMemo, useState } from "react";

/** Matches rendered at once: a native <select> with every submission (100k+) freezes the page. */
const MAX_SHOWN = 100;

interface FilePickerProps {
  /** File names, oldest first (they start with the receive stamp). */
  files: string[];
  value: string | null;
  onSelect: (file: string) => void;
}

/** A filterable list of submission files: type parts of a date or id, newest matches first. */
export function FilePicker({ files, value, onSelect }: FilePickerProps) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [highlighted, setHighlighted] = useState(0);
  const listId = useId();

  const newestFirst = useMemo(() => [...files].reverse(), [files]);
  const matches = useMemo(() => {
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
    return terms.length === 0 ? newestFirst : newestFirst.filter((file) => terms.every((term) => file.toLowerCase().includes(term)));
  }, [newestFirst, query]);
  const shown = matches.slice(0, MAX_SHOWN);

  const select = (file: string) => {
    onSelect(file);
    setOpen(false);
    setQuery("");
  };

  return (
    <div className="file-picker">
      <input
        role="combobox"
        aria-label="Search submissions"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-autocomplete="list"
        aria-activedescendant={open && shown[highlighted] ? `${listId}-${highlighted}` : undefined}
        placeholder={value ?? `Search ${files.length.toLocaleString()} submissions (date, id)...`}
        value={query}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onChange={(e) => {
          setQuery(e.target.value);
          setHighlighted(0);
          setOpen(true);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") setHighlighted((i) => Math.min(i + 1, shown.length - 1));
          else if (e.key === "ArrowUp") setHighlighted((i) => Math.max(i - 1, 0));
          else if (e.key === "Enter" && shown[highlighted]) select(shown[highlighted]);
          else if (e.key === "Escape") setOpen(false);
          else return;
          e.preventDefault();
        }}
      />
      {open && (
        <ul id={listId} className="file-picker-list" role="listbox">
          {shown.map((file, index) => (
            <li
              key={file}
              id={`${listId}-${index}`}
              role="option"
              aria-selected={index === highlighted}
              className={index === highlighted ? "highlighted" : ""}
              // mousedown, not click: the input's blur would close the list first.
              onMouseDown={(e) => {
                e.preventDefault();
                select(file);
              }}
            >
              {file}
            </li>
          ))}
          {matches.length > shown.length && (
            <li className="file-picker-more">{(matches.length - shown.length).toLocaleString()} more, type to narrow down</li>
          )}
          {matches.length === 0 && <li className="file-picker-more">No matching submission</li>}
        </ul>
      )}
    </div>
  );
}
