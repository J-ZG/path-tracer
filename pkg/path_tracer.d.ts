/* tslint:disable */
/* eslint-disable */

/**
 * Run the app on the web, opening the interactive window.
 */
export function run(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly run: () => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___js_sys_bc9ed5c38c144ee3___Array__web_sys_2138b51ec6444392___features__gen_ResizeObserver__ResizeObserver______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___js_sys_bc9ed5c38c144ee3___Function_fn_wasm_bindgen_2b3baf916ba181e6___JsValue_____wasm_bindgen_2b3baf916ba181e6___sys__Undefined___js_sys_bc9ed5c38c144ee3___Function_fn_wasm_bindgen_2b3baf916ba181e6___JsValue_____wasm_bindgen_2b3baf916ba181e6___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue__core_37ee0ce0c4cd57d5___result__Result_____wasm_bindgen_2b3baf916ba181e6___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___sys__JsNullable_wgpu_620a1a74894c880a___backend__webgpu__webgpu_sys__gen_GpuError__GpuError___core_37ee0ce0c4cd57d5___result__Result_____wasm_bindgen_2b3baf916ba181e6___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___sys__JsNullable_wgpu_620a1a74894c880a___backend__webgpu__webgpu_sys__gen_GpuError__GpuError___core_37ee0ce0c4cd57d5___result__Result_____wasm_bindgen_2b3baf916ba181e6___JsError___true__13: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___sys__JsNullable_wgpu_620a1a74894c880a___backend__webgpu__webgpu_sys__gen_GpuError__GpuError___core_37ee0ce0c4cd57d5___result__Result_____wasm_bindgen_2b3baf916ba181e6___JsError___true__14: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___core_37ee0ce0c4cd57d5___option__Option_web_sys_2138b51ec6444392___features__gen_Blob__Blob_______true_: (a: number, b: number, c: number) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__10: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__11: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__12: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__5: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__6: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__7: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke___wasm_bindgen_2b3baf916ba181e6___JsValue______true__9: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke_______true_: (a: number, b: number) => void;
    readonly wasm_bindgen_2b3baf916ba181e6___convert__closures_____invoke_______true__1_: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
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
