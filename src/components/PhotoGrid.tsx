import { Grid } from "react-window";
import type { PhotoRecord } from "../types";
import { useContainerSize } from "../hooks/useContainerSize";
import "./PhotoGrid.css";

const TILE_WIDTH = 170;
const TILE_HEIGHT = 190;

interface PhotoGridProps {
  photos: PhotoRecord[];
  onRate: (path: string, rating: number) => void;
  onOpen: (index: number) => void;
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
  photos: PhotoRecord[];
  columnCount: number;
  onRate: (path: string, rating: number) => void;
  onOpen: (index: number) => void;
}

function Cell({
  columnIndex,
  rowIndex,
  style,
  photos,
  columnCount,
  onRate,
  onOpen,
}: {
  columnIndex: number;
  rowIndex: number;
  style: React.CSSProperties;
} & CellProps) {
  const index = rowIndex * columnCount + columnIndex;
  const photo = photos[index];
  if (!photo) return null;

  return (
    <div style={style} className="photo-tile-wrapper">
      <div
        className="photo-tile"
        title={photo.filename}
        onDoubleClick={() => onOpen(index)}
      >
        {photo.thumbnail ? (
          <img src={photo.thumbnail} alt={photo.filename} draggable={false} />
        ) : (
          <div className="photo-tile-placeholder">{photo.filename}</div>
        )}
        <div className="photo-tile-caption">
          <span className="photo-tile-name">{photo.filename}</span>
          <StarRating rating={photo.rating} onRate={(r) => onRate(photo.path, r)} />
        </div>
      </div>
    </div>
  );
}

export function PhotoGrid({ photos, onRate, onOpen }: PhotoGridProps) {
  const { ref, width, height } = useContainerSize<HTMLDivElement>();
  const columnCount = Math.max(1, Math.floor(width / TILE_WIDTH));
  const rowCount = Math.ceil(photos.length / columnCount);

  return (
    <div className="photo-grid" ref={ref}>
      {width > 0 && height > 0 && (
        <Grid<CellProps>
          cellComponent={Cell}
          cellProps={{ photos, columnCount, onRate, onOpen }}
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
