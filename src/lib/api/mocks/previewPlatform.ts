import type { PreviewPlatform } from '../../models/platformContext';

let selected: PreviewPlatform = 'macos';

/**
 * The platform the preview data describes.
 *
 * Preview mode answers every command from the fixture layer, so a matrix or a
 * label has to belong to one machine. This names which one; the vocabulary
 * itself comes from the generated platform-context golden.
 */
export function previewPlatform(): PreviewPlatform {
  return selected;
}

export function setPreviewPlatform(kind: PreviewPlatform): void {
  selected = kind;
}
