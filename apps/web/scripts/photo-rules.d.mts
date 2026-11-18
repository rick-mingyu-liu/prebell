// Types for scripts/photo-rules.mjs, so the TypeScript tests can import the photo build's rules.
export function text(html: unknown): string;
export const LICENCE: RegExp;
export function licenceAllowed(name: unknown): boolean;
export function licenceUrl(url: unknown): string | null;
export function commonsPage(url: unknown): string | null;
export function authorName(html: unknown): string | null;
export function stripFlickrPlace(name: string, fromFlickr: boolean | undefined): string;
export function takenYear(raw: unknown, now?: Date): number | null;
export function captionYears(caption: string): number[];
export function headquartersNamed(caption: string, places: readonly string[]): boolean;
export function exifName(value: unknown): string;
export function exifMismatch(value: unknown, credited: readonly string[]): string | null;
export function requestNotCovered(request: string, creditName: string, licence: string): string[];
export function permissionTerms(html: unknown, licence?: string): { conditions: string; attribution: string | null };
export function originalSource(creditHtml: unknown): { url: string; title: string | null; flickr: boolean } | null;
