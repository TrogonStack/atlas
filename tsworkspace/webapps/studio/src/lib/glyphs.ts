// Canvas glyph vocabulary. Boards name the glyph they want; the renderer
// resolves the name to a vector icon.
//
// Never emoji: emoji are bitmap glyphs whose box does not follow font-size
// and which ignore text-transform and tracking, so they clip against their
// label and are resampled independently of the text as the canvas zooms.
export type Glyph = 'automation' | 'persona' | 'stream' | 'ui' | 'timeline';
