import { Grid } from "react-window";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { dragItemFor, type DragMode } from "../types";
import { useContainerSize } from "../hooks/useContainerSize";
import type { AutoScore } from "../autoScore";
import type { GridItem } from "../stacks";
import "./PhotoGrid.css";

const TILE_WIDTH = 170;
const TILE_HEIGHT = 208;

interface PhotoGridProps {
  items: GridItem[];
  onRate: (path: string, rating: number) => void;
  onOpen: (index: number) => void;
  dragMode: DragMode;
  scores: Map<string, AutoScore>;
  onToggleStack: (id: string) => void;
  onDetach: (path: string) => void;
}

export function autoTitle(a: AutoScore): string {
  const r = a.record;
  const parts = [
    `sharpness ${r.sharpness.toFixed(1)}`,
    `shake ${r.shake.toFixed(1)}`,
    `exposure ${r.exposure.toFixed(1)}`,
    `noise ${r.noise.toFixed(1)}`,
    r.horizon !== null && r.horizon_deg !== null
      ? `horizon ${r.horizon.toFixed(1)} (${Math.abs(r.horizon_deg).toFixed(1)}° off)`
      : "horizon n/a",
  ];
  if (r.ai) {
    parts.push(
      `composition ${r.ai.composition.toFixed(1)}`,
      `subject ${r.ai.subject.toFixed(1)}`,
      r.ai.people !== null ? `people ${r.ai.people.toFixed(1)}` : "no people",
      `impact ${r.ai.impact.toFixed(1)}`,
    );
  }
  return (
    parts.join(" · ") +
    (a.cappedBy ? ` — capped by ${a.cappedBy}` : "") +
    (r.ai?.summary ? "\n" + r.ai.summary : "")
  );
}

export function seriesTitle(a: AutoScore): string {
  const how = a.seriesByAi
    ? "Order decided by the local AI comparing the frames side by side"
    : "Order from the pixel analysis (sharpness, shake, exposure...)";
  return (
    `${how}, best first.` +
    (a.seriesReason ? `\n"${a.seriesReason}"` : "") +
    (a.seriesVetoed
      ? "\nThe AI preferred this frame, but it is technically much weaker than another one, so it was not made the best."
      : "")
  );
}

function StarRating({
  rating,
  onRate,
}: {
  rating: number;
  onRate: (rating: number) => void;
}) {
  return (
    <span className="star-rating">
      {[1, 2, 3, 4, 5].map((n) => (
        <button
          key={n}
          type="button"
          className={`star-button${n <= rating ? " star-filled" : ""}`}
          onClick={(e) => {
            e.stopPropagation();
            onRate(n === rating ? 0 : n);
          }}
          aria-label={`Rate ${n} stars`}
        >
          ★
        </button>
      ))}
    </span>
  );
}

interface CellProps {
  items: GridItem[];
  columnCount: number;
  onRate: (path: string, rating: number) => void;
  onOpen: (index: number) => void;
  dragMode: DragMode;
  scores: Map<string, AutoScore>;
  onToggleStack: (id: string) => void;
  onDetach: (path: string) => void;
}

function Cell({
  columnIndex,
  rowIndex,
  style,
  items,
  columnCount,
  onRate,
  onOpen,
  dragMode,
  scores,
  onToggleStack,
  onDetach,
}: {
  columnIndex: number;
  rowIndex: number;
  style: React.CSSProperties;
} & CellProps) {
  const index = rowIndex * columnCount + columnIndex;
  const item = items[index];
  if (!item) return null;
  const { photo, stack } = item;
  const auto = photo.rated ? undefined : scores.get(photo.path);
  const collapsedStack = stack !== null && !stack.expanded;
  const tileClass = [
    "photo-tile",
    stack && "in-stack",
    collapsedStack && "stack-collapsed",
    stack?.expanded && stack.isCover && "stack-cover",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div style={style} className="photo-tile-wrapper">
      <div
        className={tileClass}
        title={photo.filename}
        onDoubleClick={() => onOpen(index)}
      >
        {photo.thumbnail ? (
          <img
            src={photo.thumbnail}
            alt={photo.filename}
            draggable={true}
            onDragStart={(e) => {
              e.preventDefault();
              startDrag({ item: dragItemFor(photo, dragMode), icon: photo.path });
            }}
          />
        ) : (
          <div className="photo-tile-placeholder">{photo.filename}</div>
        )}
        {stack && (
          <div className="stack-controls">
            {stack.isCover ? (
              <button
                className="stack-toggle"
                title={stack.expanded ? "Collapse this stack" : "Expand to see all frames"}
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleStack(stack.id);
                }}
              >
                {stack.expanded ? "▾ Collapse" : `▣ ${stack.size} frames`}
              </button>
            ) : null}
            {stack.expanded && (
              <button
                className="stack-remove"
                title="Take this photo out of the stack (view only, no files are changed)"
                onClick={(e) => {
                  e.stopPropagation();
                  onDetach(photo.path);
                }}
              >
                ✕ Remove from stack
              </button>
            )}
          </div>
        )}
        <div className="photo-tile-caption">
          <div className="photo-tile-name-row">
            <span className="photo-tile-name">{photo.filename}</span>
            {photo.raw_path && <span className="raw-badge">RAW+JPG</span>}
          </div>
          {auto && (
            <div className="auto-badges">
              <span className="auto-badge" title={autoTitle(auto)}>
                A {auto.score.toFixed(1)}
              </span>
              {auto.seriesSize > 1 && (
                <span
                  className={`series-badge${auto.seriesBest ? " series-best" : ""}`}
                  title={seriesTitle(auto)}
                >
                  {auto.seriesByAi ? "🧠 " : ""}
                  {auto.seriesBest ? "★ best" : ""} {auto.seriesPos}/{auto.seriesSize}
                  {auto.seriesVetoed ? " ⚠" : ""}
                </span>
              )}
            </div>
          )}
          <StarRating rating={photo.rating} onRate={(r) => onRate(photo.path, r)} />
        </div>
      </div>
    </div>
  );
}

export function PhotoGrid({
  items,
  onRate,
  onOpen,
  dragMode,
  scores,
  onToggleStack,
  onDetach,
}: PhotoGridProps) {
  const { ref, width, height } = useContainerSize<HTMLDivElement>();
  const columnCount = Math.max(1, Math.floor(width / TILE_WIDTH));
  const rowCount = Math.ceil(items.length / columnCount);

  return (
    <div className="photo-grid" ref={ref}>
      {width > 0 && height > 0 && (
        <Grid<CellProps>
          cellComponent={Cell}
          cellProps={{ items, columnCount, onRate, onOpen, dragMode, scores, onToggleStack, onDetach }}
          columnCount={columnCount}
          columnWidth={TILE_WIDTH}
          rowCount={Math.max(rowCount, 1)}
          rowHeight={TILE_HEIGHT}
          style={{ width, height }}
          overscanCount={2}
        />
      )}
    </div>
  );
}
