import { useState, useEffect } from "react";

export type MapImageStatus = "idle" | "loading" | "loaded" | "error";

export interface MapImageState {
  image: HTMLImageElement | null;
  status: MapImageStatus;
}

/**
 * Decoded map art, shared across mounts so toggling the backdrop, scrubbing the
 * timeline, or moving back into a zone already visited never re-fetches.
 */
const cache = new Map<string, HTMLImageElement>();

function startLoad(url: string): HTMLImageElement {
  const img = new Image();
  // Local assets are same-origin; the CDN sends Access-Control-Allow-Origin: *.
  // Either way this keeps the canvas untainted for pixel readback.
  img.crossOrigin = "anonymous";
  img.src = url;
  return img;
}

/**
 * Warm the cache for every map a session visits.
 *
 * Without this, crossing a zone boundary during playback drops the backdrop to
 * the bare grid while the next map downloads. Preloading makes the swap instant.
 */
export function preloadMapArt(urls: string[]): void {
  for (const url of urls) {
    if (cache.has(url)) continue;
    const img = startLoad(url);
    img.onload = () => cache.set(url, img);
  }
}

export function useMapImage(url: string | null): MapImageState {
  const [state, setState] = useState<MapImageState>({
    image: null,
    status: "idle",
  });

  useEffect(() => {
    if (!url) {
      setState({ image: null, status: "idle" });
      return;
    }

    const cached = cache.get(url);
    if (cached) {
      setState({ image: cached, status: "loaded" });
      return;
    }

    // Guards against a slow response for a previous url overwriting a newer one.
    let cancelled = false;
    setState({ image: null, status: "loading" });

    const img = startLoad(url);
    img.onload = () => {
      cache.set(url, img);
      if (!cancelled) setState({ image: img, status: "loaded" });
    };
    img.onerror = () => {
      if (!cancelled) setState({ image: null, status: "error" });
    };

    return () => {
      cancelled = true;
    };
  }, [url]);

  // Resolve cache hits during render rather than waiting for the effect, so a
  // swap to an already-loaded map paints the new art on the very first frame
  // instead of briefly showing the outgoing zone's map.
  if (url) {
    const cached = cache.get(url);
    if (cached && state.image !== cached) {
      return { image: cached, status: "loaded" };
    }
  }

  return state;
}
