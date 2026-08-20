//! # sysml-service-macros — Proc-macro crate for `#[service_command]`
//!
//! Generates request types, `CommandMeta` constants, `ServiceCommand` trait
//! implementations, and `inventory` registrations from annotated service methods.
//!
//! ## Usage
//!
//! Apply `#[service_impl]` to the `impl SysmlService` block. Annotate individual
//! methods with `#[service_command(...)]` — the container macro extracts these
//! annotations and emits companion items *after* the impl block.
//!
//! ```rust,ignore
//! use sysml_service_macros::service_impl;
//!
//! #[service_impl]
//! impl SysmlService {
//!     #[service_command(
//!         name = "sysml.find",
//!         category = Query,
//!         description = "Find elements by name pattern",
//!         returns = "Vec<Element>",
//!     )]
//!     pub fn find(
//!         &self,
//!         #[doc = "URI of the loaded model"] uri: &str,
//!         #[doc = "Name pattern (substring match)"] pattern: &str,
//!     ) -> Result<Vec<Element>, ServiceError> {
//!         // ... implementation ...
//!     }
//! }
//! ```
//!
//! This generates (outside the impl block):
//! - `FindRequest` — a deserializable request struct with wire types
//! - `FIND_META` — a `CommandMeta` constant with full parameter metadata
//! - `FindCommand` — a zero-sized struct implementing `ServiceCommand`
//! - An `inventory::submit!` block for dynamic command discovery

mod codegen;
mod parse;
mod type_mapping;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ImplItem, ItemImpl};

use parse::{CommandAttrs, MethodInfo};

/// Container attribute macro for `impl SysmlService` blocks.
///
/// Scans methods for `#[service_command(...)]` annotations, strips them from the
/// output, and emits companion types and inventory registrations after the impl
/// block. Non-annotated methods pass through unchanged.
#[proc_macro_attribute]
pub fn service_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut impl_block = parse_macro_input!(item as ItemImpl);

    let mut generated_items: Vec<proc_macro2::TokenStream> = Vec::new();

    for item in &mut impl_block.items {
        if let ImplItem::Fn(method) = item {
            // Find #[service_command(...)] attribute on this method
            let sc_idx = method
                .attrs
                .iter()
                .position(|attr| attr.path().is_ident("service_command"));

            if let Some(idx) = sc_idx {
                // Safety: `idx` was just returned by `position()` on this same vec
                #[allow(clippy::indexing_slicing)]
                let attr = &method.attrs[idx];
                let attr_tokens = match &attr.meta {
                    syn::Meta::List(list) => list.tokens.clone(),
                    _ => {
                        return syn::Error::new_spanned(
                            attr,
                            "#[service_command] expects parenthesized arguments, \
                             e.g. #[service_command(name = \"...\", ...)]",
                        )
                        .to_compile_error()
                        .into();
                    }
                };

                let cmd_attrs = match syn::parse2::<CommandAttrs>(attr_tokens) {
                    Ok(a) => a,
                    Err(e) => return e.to_compile_error().into(),
                };

                let method_info = match MethodInfo::from_method(method) {
                    Ok(info) => info,
                    Err(e) => return e.to_compile_error().into(),
                };

                match codegen::generate(&cmd_attrs, &method_info) {
                    Ok(tokens) => generated_items.push(tokens),
                    Err(e) => return e.to_compile_error().into(),
                }

                // Strip #[service_command(...)] from the method so the compiler
                // doesn't see it in the output.
                method.attrs.remove(idx);

                // Strip #[doc = "..."] from function parameters — they were used
                // by MethodInfo for parameter descriptions but are not valid on
                // function parameters in the compiler output.
                for input in &mut method.sig.inputs {
                    if let syn::FnArg::Typed(pat_type) = input {
                        pat_type.attrs.retain(|attr| !attr.path().is_ident("doc"));
                    }
                }
            }
        }
    }

    let output = quote! {
        #impl_block
        #(#generated_items)*
    };

    output.into()
}

/// Marker attribute for annotating service methods within a `#[service_impl]` block.
///
/// This is a no-op when invoked standalone — `#[service_impl]` processes and
/// consumes these annotations before the compiler expands them. It is defined as
/// a proc macro attribute so that it can be imported and used syntactically.
#[proc_macro_attribute]
pub fn service_command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
