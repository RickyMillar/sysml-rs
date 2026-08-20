/**
 * Rendered in the DetailPanel when no node is focused. Guides the
 * user toward clicking a tree row.
 */
export function EmptyDetail({ testIdPrefix }: { testIdPrefix: string }) {
  return (
    <div
      data-testid={`${testIdPrefix}-empty`}
      className="flex flex-col items-center justify-center gap-2 p-4 h-full"
      style={{ color: 'var(--outline)', minHeight: 80 }}
    >
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{ fontSize: 24, opacity: 0.7 }}
      >
        ads_click
      </span>
      <span style={{ fontSize: 11, textAlign: 'center' }}>
        Click any row above to inspect it.
      </span>
    </div>
  );
}
