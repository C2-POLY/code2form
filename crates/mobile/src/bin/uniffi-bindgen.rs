//! Writes the Swift bindings of the library: run by
//! scripts/ios-xcframework.sh.

fn main() {
    uniffi::uniffi_bindgen_main()
}
