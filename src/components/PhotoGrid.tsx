import type { PhotoRecord } from "../types";
import "./PhotoGrid.css";

interface PhotoGridProps {
  photos: PhotoRecord[];
  onRate: (path: string, rating: number) => void;
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

export function PhotoGrid({ photos, onRate }: PhotoGridProps) {
  return (
    <div className="photo-grid">
      {photos.map((photo) => (
        <div className="photo-tile" key={photo.path} title={photo.filename}>
          {photo.thumbnail ? (
            <img src={photo.thumbnail} alt={photo.filename} draggable={false} />
          ) : (
            <div className="photo-tile-placeholder">{photo.filename}</div>
          )}
          <div className="photo-tile-caption">
            <span className="photo-tile-name">{photo.filename}</span>
            <StarRating
              rating={photo.rating}
              onRate={(rating) => onRate(photo.path, rating)}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
