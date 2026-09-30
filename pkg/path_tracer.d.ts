/* tslint:disable */
/* eslint-disable */

/**
 * The main function that runs the app, called by both native and web targets.
 */
export function run(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly run: () => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___js_sys_23c4f7ff1cef9bb3___Array__web_sys_3eebdec999cdeb5___features__gen_ResizeObserver__ResizeObserver______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___js_sys_23c4f7ff1cef9bb3___Function_fn_wasm_bindgen_74d33da26aa83798___JsValue_____wasm_bindgen_74d33da26aa83798___sys__Undefined___js_sys_23c4f7ff1cef9bb3___Function_fn_wasm_bindgen_74d33da26aa83798___JsValue_____wasm_bindgen_74d33da26aa83798___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue__core_f0fd674eaa06beef___result__Result_____wasm_bindgen_74d33da26aa83798___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___core_f0fd674eaa06beef___option__Option_web_sys_3eebdec999cdeb5___features__gen_Blob__Blob_______true_: (a: number, b: number, c: number) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__10: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__11: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__12: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__1_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__6: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__7: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__8: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke___wasm_bindgen_74d33da26aa83798___JsValue______true__9: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke_______true_: (a: number, b: number) => void;
    readonly wasm_bindgen_74d33da26aa83798___convert__closures_____invoke_______true__1_: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
