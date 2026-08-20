/**
 * Export utilities — CSV, JSON, and PNG download helpers.
 *
 * Per ADR-005: CSV for waveforms, JSON for full archives, PNG for charts.
 * Uses the Blob + URL.createObjectURL + <a download> pattern (no heavy deps).
 */

// ── Helpers ──────────────────────────────────────────────────────────

function triggerDownload(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  // Cleanup after a tick so the browser can finish the download
  setTimeout(() => {
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, 100);
}

// ── CSV ──────────────────────────────────────────────────────────────

/**
 * Generate a CSV file and trigger a browser download.
 *
 * @param headers  Column header names
 * @param rows     2D array of values (one inner array per row)
 * @param filename Download filename (should end in `.csv`)
 */
export function exportCSV(headers: string[], rows: (string | number)[][], filename: string): void {
  const lines: string[] = [headers.join(',')];
  for (const row of rows) {
    lines.push(row.map((cell) => {
      const s = String(cell);
      // Quote cells containing commas or quotes
      return s.includes(',') || s.includes('"') ? `"${s.replace(/"/g, '""')}"` : s;
    }).join(','));
  }
  const blob = new Blob([lines.join('\n')], { type: 'text/csv;charset=utf-8' });
  triggerDownload(blob, filename);
}

// ── JSON ─────────────────────────────────────────────────────────────

/**
 * Serialize data as pretty-printed JSON and trigger a browser download.
 */
export function exportJSON(data: unknown, filename: string): void {
  const text = JSON.stringify(data, null, 2);
  const blob = new Blob([text], { type: 'application/json;charset=utf-8' });
  triggerDownload(blob, filename);
}

// ── PNG (SVG-based charts) ───────────────────────────────────────────

/**
 * Export an HTML element as a PNG image.
 *
 * For SVG-based charts (our default), this serializes the first child
 * `<svg>` to a data URL, draws it onto an offscreen canvas, and
 * triggers a blob download. Falls back to canvas.toBlob if the element
 * itself is a `<canvas>`.
 */
export function exportPNG(element: HTMLElement, filename: string): void {
  // Fast path: element is already a <canvas>
  if (element instanceof HTMLCanvasElement) {
    element.toBlob((blob) => {
      if (blob) triggerDownload(blob, filename);
    });
    return;
  }

  // SVG path: find the first <svg> child (or the element itself)
  const svg = element.tagName === 'svg'
    ? element
    : element.querySelector('svg');

  if (!svg) {
    console.warn('[export] No <svg> or <canvas> found in element');
    return;
  }

  // Clone the SVG so we can inject computed styles without mutating the DOM
  const clone = svg.cloneNode(true) as SVGSVGElement;

  // Ensure width/height attributes exist for the image renderer
  const box = svg.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const w = Math.ceil(box.width * dpr);
  const h = Math.ceil(box.height * dpr);

  if (!clone.getAttribute('width')) clone.setAttribute('width', String(box.width));
  if (!clone.getAttribute('height')) clone.setAttribute('height', String(box.height));
  clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');

  const serialized = new XMLSerializer().serializeToString(clone);
  const dataUrl = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(serialized);

  const img = new Image();
  img.onload = () => {
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    ctx.drawImage(img, 0, 0, box.width, box.height);
    canvas.toBlob((blob) => {
      if (blob) triggerDownload(blob, filename);
    });
  };
  img.src = dataUrl;
}
