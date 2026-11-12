import type { ReactNode } from "react";

import { cn } from "./cn";

export type Column<Row> = {
  key: string;
  header: string;
  cell: (row: Row) => ReactNode;
  /** Numbers line up on the right. */
  numeric?: boolean;
};

/**
 * A data table with a caption (so it has a name), column headers tied to their cells, and a
 * wrapper that scrolls sideways on a narrow screen instead of squeezing the layout.
 */
export function Table<Row>({
  caption,
  columns,
  rows,
  rowKey,
  empty,
}: {
  caption: string;
  columns: Column<Row>[];
  rows: Row[];
  rowKey: (row: Row) => string;
  empty: string;
}) {
  return (
    <div className="overflow-x-auto rounded-card border border-border" tabIndex={0} role="region" aria-label={caption}>
      <table className="w-full border-collapse text-left text-sm">
        <caption className="sr-only">{caption}</caption>
        <thead className="bg-surface-strong">
          <tr>
            {columns.map((column) => (
              <th key={column.key} scope="col" className={cn("px-4 py-3 font-semibold", column.numeric && "text-right")}>
                {column.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={columns.length} className="px-4 py-6 text-center text-text-muted">
                {empty}
              </td>
            </tr>
          ) : (
            rows.map((row) => (
              <tr key={rowKey(row)} className="border-t border-border">
                {columns.map((column) => (
                  <td key={column.key} className={cn("px-4 py-3", column.numeric && "text-right tabular-nums")}>
                    {column.cell(row)}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}
