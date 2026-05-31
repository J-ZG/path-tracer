/// On web, [`Result::expect()`] or [`Option::unwrap()`] both aren't recommended.
///
/// Instead, [`wasm_bindgen`]'s
/// [`wasm_bindgen::UnwrapThrowExt::expect_throw`] or
/// [`wasm_bindgen::UnwrapThrowExt::unwrap_throw`] are recommended,
/// which throw JavaScript errors.
///
/// However, always writing two versions of the same code, one with
/// [`Result::expect()`] and one with
/// [`wasm_bindgen::UnwrapThrowExt::expect_throw`], is tedious.
///
/// So, always use [`Self::expect_universal`] defined here.
pub trait ExpectUniversal<T> {
    fn expect_universal(self, message: &str) -> T;
}

#[cfg(target_arch = "wasm32")]
impl<T> ExpectUniversal<T> for Option<T> {
    fn expect_universal(self, message: &str) -> T {
        use wasm_bindgen::UnwrapThrowExt;
        self.expect_throw(message)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> ExpectUniversal<T> for Option<T> {
    fn expect_universal(self, message: &str) -> T {
        self.expect(message)
    }
}

#[cfg(target_arch = "wasm32")]
impl<T, E: core::fmt::Debug> ExpectUniversal<T> for Result<T, E> {
    fn expect_universal(self, message: &str) -> T {
        use wasm_bindgen::UnwrapThrowExt;
        self.expect_throw(message)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T, E: core::fmt::Debug> ExpectUniversal<T> for Result<T, E> {
    fn expect_universal(self, message: &str) -> T {
        self.expect(message)
    }
}
