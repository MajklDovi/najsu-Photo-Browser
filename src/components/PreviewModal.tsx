import { useEffect } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { dragItemFor, type DragMode, type PhotoRecord } from "../types";
import "./PreviewModal.css";

interface PreviewModalProps {
  photos: PhotoRecord[];
  index: number;
  onClose: () => void;
  onNavigate: (index: number) => void;
  onRate: (path: string, rating: number) => void;
  dragMode: DragMode;
}

export function PreviewModal({
  photos,
  index,
  onClose,
  onNavigate,
  onRate,
  dragMode,
}: PreviewModalProps) {
  const photo = photos[index];

  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        onClose();
      } else if (e.key === "ArrowLeft") {
        onNavigate(index - 1);
      } else if (e.key === "ArrowRight") {
        onNavigate(index + 1);
      } else if (photo) {
        // Match the physical key position (e.code), not the character it produces
        // (e.key), so number-row rating still works on layouts where that row
        // types accented letters (e.g. Czech QWERTZ: ě š č ř ž ý á í é) unless
        // Shift is held.
        const digitMatch = /^Digit([0-5])$/.exec(e.code);
        if (digitMatch) {
          onRate(photo.path, Number(digitMatch[1]));
        }
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [index, photo, onClose, onNavigate, onRate]);

  if (!photo) return null;

  return (
    <div className="preview-overlay" onClick={onClose}>
      <div className="preview-content" onClick={(e) => e.stopPropagation()}>
        <button className="preview-close" onClick={onClose} aria-label="Close preview">
          ✕
        </button>

        <button
          className="preview-nav preview-nav-prev"
          onClick={() => onNavigate(index - 1)}
          disabled={index <= 0}
          aria-label="Previous photo"
        >
          ‹
        </button>

        <img
          className="preview-image"
          src={convertFileSrc(photo.path)}
          alt={photo.filename}
          draggable={true}
          onDragStart={(e) => {
            e.preventDefault();
            startDrag({ item: dragItemFor(photo, dragMode), icon: photo.path });
          }}
        />

        <button
          className="preview-nav preview-nav-next"
          onClick={() => onNavigate(index + 1)}
          disabled={index >= photos.length - 1}
          aria-label="Next photo"
        >
          ›
        </button>

        <div className="preview-footer">
          <span className="preview-filename">{photo.filename}</span>
          {photo.raw_path && <span className="raw-badge">RAW+JPG</span>}
          <span className="preview-stars">
            {[1, 2, 3, 4, 5].map((n) => (
              <button
                key={n}
                type="button"
                className={`preview-star${n <= photo.rating ? " preview-star-filled" : ""}`}
                onClick={() => onRate(photo.path, n === photo.rating ? 0 : n)}
                aria-label={`Rate ${n} stars`}
              >
                ★
              </button>
            ))}
          </span>
          <span className="preview-position">
            {index + 1} / {photos.length}
          </span>
        </div>
      </div>
    </div>
  );
}
