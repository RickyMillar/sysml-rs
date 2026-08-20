/**
 * Backward-compatible re-export shim.
 *
 * The implementation now lives in `./transport`, which detects the active
 * backend (Tauri invoke vs REST fetch) once at startup.  All imports from
 * this module continue to work; new code should import from `./transport`.
 */
export { httpGet, httpPost, httpPostText, httpDelete, ApiError } from './transport';
