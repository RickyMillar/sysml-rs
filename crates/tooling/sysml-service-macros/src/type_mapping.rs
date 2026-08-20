//! Rust type to wire type mappings for request struct generation.
//!
//! Handles the translation between method parameter types (which may include
//! references, slices, etc.) and the owned types used in generated request
//! structs, plus the conversion expressions to go from wire back to Rust.

use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::{GenericArgument, PathArguments, Type, TypePath, TypeReference};

/// Maps a method parameter type to its corresponding request struct field type.
///
/// For example, `&str` becomes `String`, `Option<&ElementKind>` becomes `Option<String>`,
/// `&[(String, String)]` becomes `Vec<(String, String)>`.
pub fn wire_type(ty: &Type) -> Result<TokenStream, String> {
    match ty {
        // Reference types: &str, &T, &[T], &Path, etc.
        Type::Reference(TypeReference { elem, .. }) => wire_type_for_referenced(elem),

        // Option<T> — unwrap and recurse
        Type::Path(tp) if is_option_path(tp) => {
            let inner = option_inner_type(tp)
                .ok_or_else(|| "Option type missing inner type argument".to_owned())?;
            let inner_wire = wire_type(inner)?;
            Ok(quote! { Option<#inner_wire> })
        }

        // Primitive owned types: usize, bool, String, etc.
        // Also: domain types that impl Deserialize (SnapshotMeta, Breakpoint) → serde_json::Value
        Type::Path(tp) => {
            let type_name = path_ident_string(tp);
            match type_name.as_deref() {
                Some("usize") => Ok(quote! { usize }),
                Some("bool") => Ok(quote! { bool }),
                Some("u32") => Ok(quote! { u32 }),
                Some("u64") => Ok(quote! { u64 }),
                Some("i32") => Ok(quote! { i32 }),
                Some("i64") => Ok(quote! { i64 }),
                Some("f64") => Ok(quote! { f64 }),
                Some("String") => Ok(quote! { String }),
                Some("SnapshotMeta") => Ok(quote! { serde_json::Value }),
                Some("Breakpoint") => Ok(quote! { serde_json::Value }),
                Some("BatchFilter") => Ok(quote! { serde_json::Value }),
                _ => Err(format!(
                    "unsupported parameter type: `{}`",
                    quote!(#ty)
                )),
            }
        }

        // Tuple types: (A, B, ...)
        Type::Tuple(tuple) => {
            let inner: Vec<TokenStream> = tuple
                .elems
                .iter()
                .map(wire_type)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(quote! { (#(#inner),*) })
        }

        _ => Err(format!("unsupported parameter type: `{}`", quote!(#ty))),
    }
}

/// Maps a referenced type (`&T` already stripped of the `&`) to a wire type.
fn wire_type_for_referenced(elem: &Type) -> Result<TokenStream, String> {
    match elem {
        // &str → String
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("str") => {
            Ok(quote! { String })
        }

        // &Path or &std::path::Path → String
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Path") => {
            Ok(quote! { String })
        }

        // &String → String
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("String") => {
            Ok(quote! { String })
        }

        // &ElementId, &ElementKind, &RelationshipKind, &ProjectId, &CommitId → String
        Type::Path(tp)
            if matches!(
                path_ident_string(tp).as_deref(),
                Some("ElementId") | Some("ElementKind") | Some("RelationshipKind")
                    | Some("ProjectId") | Some("CommitId")
            ) =>
        {
            Ok(quote! { String })
        }

        // &HashSet<String> → Vec<String>
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("HashSet") => {
            let inner = first_generic_arg(tp)
                .ok_or_else(|| "HashSet missing type argument".to_owned())?;
            let inner_wire = wire_type(inner)?;
            Ok(quote! { Vec<#inner_wire> })
        }

        // &ModelGraph → serde_json::Value (JSON pass-through)
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("ModelGraph") => {
            Ok(quote! { serde_json::Value })
        }

        // &Value (serde_json::Value) → serde_json::Value (JSON pass-through, no deserialize)
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Value") => {
            Ok(quote! { serde_json::Value })
        }

        // &Vec<T> → Vec<T_wire>
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Vec") => {
            let inner = first_generic_arg(tp)
                .ok_or_else(|| "Vec missing type argument".to_owned())?;
            let inner_wire = wire_type(inner)?;
            Ok(quote! { Vec<#inner_wire> })
        }

        // &[T] (slice) → Vec<T_wire>
        Type::Slice(slice) => {
            let inner_wire = wire_type(&slice.elem)?;
            Ok(quote! { Vec<#inner_wire> })
        }

        // Fallback for other &T — try to map T itself
        Type::Path(tp) => {
            // For unknown reference types, try treating as owned
            let type_name = path_ident_string(tp);
            match type_name.as_deref() {
                Some("usize") | Some("bool") | Some("u32") | Some("u64") | Some("i32")
                | Some("i64") | Some("f64") => wire_type(elem),
                _ => Err(format!(
                    "unsupported referenced parameter type: `&{}`",
                    quote!(#elem)
                )),
            }
        }

        _ => Err(format!(
            "unsupported referenced parameter type: `&{}`",
            quote!(#elem)
        )),
    }
}

/// Returns the type string used in `ParamMeta.ty`.
///
/// For example: `&str` → `"string"`, `Option<&ElementKind>` → `"ElementKind?"`,
/// `&[(String, String)]` → `"[(string, string)]"`.
pub fn type_string(ty: &Type) -> String {
    match ty {
        Type::Reference(TypeReference { elem, .. }) => type_string_for_referenced(elem),

        Type::Path(tp) if is_option_path(tp) => {
            if let Some(inner) = option_inner_type(tp) {
                let base = type_string(inner);
                // Strip trailing ? if already present (from nested Option) then add ?
                let base = base.trim_end_matches('?');
                format!("{}?", base)
            } else {
                "unknown?".to_owned()
            }
        }

        Type::Path(tp) => {
            let name = path_ident_string(tp);
            match name.as_deref() {
                Some("usize") => "usize".to_owned(),
                Some("bool") => "bool".to_owned(),
                Some("u32") => "u32".to_owned(),
                Some("u64") => "u64".to_owned(),
                Some("i32") => "i32".to_owned(),
                Some("i64") => "i64".to_owned(),
                Some("f64") => "f64".to_owned(),
                Some("String") => "string".to_owned(),
                Some("SnapshotMeta") => "SnapshotMeta".to_owned(),
                Some("Breakpoint") => "Breakpoint".to_owned(),
                Some("BatchFilter") => "BatchFilter".to_owned(),
                Some(other) => other.to_owned(),
                None => "unknown".to_owned(),
            }
        }

        Type::Tuple(tuple) => {
            let parts: Vec<String> = tuple.elems.iter().map(type_string).collect();
            format!("({})", parts.join(", "))
        }

        _ => "unknown".to_owned(),
    }
}

fn type_string_for_referenced(elem: &Type) -> String {
    match elem {
        Type::Path(tp) => {
            let name = path_ident_string(tp);
            match name.as_deref() {
                Some("str") | Some("String") => "string".to_owned(),
                Some("Path") => "string".to_owned(),
                Some("ProjectId") => "string".to_owned(),
                Some("CommitId") => "string".to_owned(),
                Some("ModelGraph") => "ModelGraph".to_owned(),
                Some("Value") => "json".to_owned(),
                Some("HashSet") => {
                    if let Some(inner) = first_generic_arg(tp) {
                        format!("Set<{}>", type_string(inner))
                    } else {
                        "Set<unknown>".to_owned()
                    }
                }
                Some("Vec") => {
                    if let Some(inner) = first_generic_arg(tp) {
                        format!("[{}]", type_string(inner))
                    } else {
                        "[unknown]".to_owned()
                    }
                }
                Some(other) => other.to_owned(),
                None => "unknown".to_owned(),
            }
        }

        Type::Slice(slice) => {
            let inner = type_string(&slice.elem);
            format!("[{}]", inner)
        }

        _ => "unknown".to_owned(),
    }
}

/// Returns whether the given method parameter type is `Option<_>`.
pub fn is_optional(ty: &Type) -> bool {
    match ty {
        Type::Path(tp) => is_option_path(tp),
        _ => false,
    }
}

/// Generates the conversion expression from the wire-type request field
/// back to the Rust type expected by the original method.
///
/// `param_name` is the identifier of the field on the request struct (e.g. `uri`).
/// `rust_type` is the original parameter type from the method signature.
///
/// Returns a token stream for the expression, plus an optional `let` binding
/// that must be emitted before the method call (for types needing intermediate storage).
pub fn conversion_expr(
    param_name: &Ident,
    rust_type: &Type,
) -> Result<ConversionResult, String> {
    match rust_type {
        // === Reference types ===
        Type::Reference(TypeReference { elem, .. }) => {
            conversion_for_referenced(param_name, elem)
        }

        // === Option<&T> ===
        Type::Path(tp) if is_option_path(tp) => {
            let inner = option_inner_type(tp)
                .ok_or_else(|| "Option missing inner type".to_owned())?;
            conversion_for_option(param_name, inner)
        }

        // === Owned primitives: usize, bool, etc. ===
        Type::Path(tp) => {
            let name = path_ident_string(tp);
            match name.as_deref() {
                Some("usize") | Some("bool") | Some("u32") | Some("u64") | Some("i32")
                | Some("i64") | Some("f64") | Some("String") => Ok(ConversionResult {
                    bindings: quote! {},
                    expr: quote! { req.#param_name },
                }),
                // SnapshotMeta: deserialize from serde_json::Value
                Some("SnapshotMeta") => {
                    let binding_name = Ident::new(
                        &format!("{}_parsed", param_name),
                        param_name.span(),
                    );
                    Ok(ConversionResult {
                        bindings: quote! {
                            let #binding_name: sysml_store::SnapshotMeta = serde_json::from_value(req.#param_name.clone())
                                .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                        },
                        expr: quote! { #binding_name },
                    })
                }
                // Breakpoint: deserialize from serde_json::Value
                Some("Breakpoint") => {
                    let binding_name = Ident::new(
                        &format!("{}_parsed", param_name),
                        param_name.span(),
                    );
                    Ok(ConversionResult {
                        bindings: quote! {
                            let #binding_name: sysml_runtime::breakpoint::Breakpoint = serde_json::from_value(req.#param_name.clone())
                                .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                        },
                        expr: quote! { #binding_name },
                    })
                }
                // BatchFilter: deserialize from serde_json::Value
                Some("BatchFilter") => {
                    let binding_name = Ident::new(
                        &format!("{}_parsed", param_name),
                        param_name.span(),
                    );
                    Ok(ConversionResult {
                        bindings: quote! {
                            let #binding_name: crate::batch::BatchFilter = serde_json::from_value(req.#param_name.clone())
                                .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                        },
                        expr: quote! { #binding_name },
                    })
                }
                _ => Err(format!(
                    "unsupported parameter type for conversion: `{}`",
                    quote!(#rust_type)
                )),
            }
        }

        // Tuple types pass through directly
        Type::Tuple(_) => Ok(ConversionResult {
            bindings: quote! {},
            expr: quote! { req.#param_name },
        }),

        _ => Err(format!(
            "unsupported parameter type for conversion: `{}`",
            quote!(#rust_type)
        )),
    }
}

/// Result of generating a conversion expression.
pub struct ConversionResult {
    /// Any `let` bindings that must precede the method call.
    pub bindings: TokenStream,
    /// The expression to pass as the method argument.
    pub expr: TokenStream,
}

fn conversion_for_referenced(
    param_name: &Ident,
    elem: &Type,
) -> Result<ConversionResult, String> {
    match elem {
        // &str → &req.field
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("str") => {
            Ok(ConversionResult {
                bindings: quote! {},
                expr: quote! { &req.#param_name },
            })
        }

        // &Path → Path::new(&req.field)
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Path") => {
            let binding_name = Ident::new(
                &format!("{}_path", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name = std::path::Path::new(&req.#param_name);
                },
                expr: quote! { #binding_name },
            })
        }

        // &String → &req.field
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("String") => {
            Ok(ConversionResult {
                bindings: quote! {},
                expr: quote! { &req.#param_name },
            })
        }

        // &ElementId → ElementId::from(req.field.clone()), then &ref
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("ElementId") => {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name = sysml_id::ElementId::from_string(req.#param_name.clone());
                },
                expr: quote! { &#binding_name },
            })
        }

        // &ProjectId → ProjectId::new(req.field.clone()), then &ref
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("ProjectId") => {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name = sysml_id::ProjectId::new(req.#param_name.clone());
                },
                expr: quote! { &#binding_name },
            })
        }

        // &CommitId → CommitId::new(req.field.clone()), then &ref
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("CommitId") => {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name = sysml_id::CommitId::new(req.#param_name.clone());
                },
                expr: quote! { &#binding_name },
            })
        }

        // &Value (serde_json::Value) → &req.field directly (no deserialize)
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Value") => {
            Ok(ConversionResult {
                bindings: quote! {},
                expr: quote! { &req.#param_name },
            })
        }

        // &ModelGraph → deserialize from JSON Value, then &ref
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("ModelGraph") => {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name: sysml_core::ModelGraph = serde_json::from_value(req.#param_name.clone())
                        .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                },
                expr: quote! { &#binding_name },
            })
        }

        // &ElementKind → serde_json::from_value, then &ref
        Type::Path(tp)
            if matches!(
                path_ident_string(tp).as_deref(),
                Some("ElementKind") | Some("RelationshipKind")
            ) =>
        {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name: #elem = serde_json::from_value(
                        serde_json::Value::String(req.#param_name.clone())
                    ).map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                },
                expr: quote! { &#binding_name },
            })
        }

        // &HashSet<String> → collect from Vec, then &ref
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("HashSet") => {
            let binding_name = Ident::new(
                &format!("{}_set", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name: #elem = req.#param_name.into_iter().collect();
                },
                expr: quote! { &#binding_name },
            })
        }

        // &Vec<T> → &req.field
        Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Vec") => {
            Ok(ConversionResult {
                bindings: quote! {},
                expr: quote! { &req.#param_name },
            })
        }

        // &[T] (slice) → &req.field (Vec derefs to slice)
        Type::Slice(_) => Ok(ConversionResult {
            bindings: quote! {},
            expr: quote! { &req.#param_name },
        }),

        _ => Err(format!(
            "unsupported referenced type for conversion: `&{}`",
            quote!(#elem)
        )),
    }
}

fn conversion_for_option(
    param_name: &Ident,
    inner: &Type,
) -> Result<ConversionResult, String> {
    match inner {
        // Option<&str> → req.field.as_deref()
        Type::Reference(TypeReference { elem, .. })
            if matches!(elem.as_ref(), Type::Path(tp) if path_ident_string(tp).as_deref() == Some("str")) =>
        {
            Ok(ConversionResult {
                bindings: quote! {},
                expr: quote! { req.#param_name.as_deref() },
            })
        }

        // Option<&ElementKind> or similar serde types → parse then .as_ref()
        Type::Reference(TypeReference { elem, .. })
            if matches!(elem.as_ref(), Type::Path(tp) if matches!(
                path_ident_string(tp).as_deref(),
                Some("ElementKind") | Some("RelationshipKind")
            )) =>
        {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            let inner_type = elem.as_ref();
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name: Option<#inner_type> = req.#param_name
                        .as_deref()
                        .map(|s| serde_json::from_value(serde_json::Value::String(s.to_string())))
                        .transpose()
                        .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                },
                expr: quote! { #binding_name.as_ref() },
            })
        }

        // Option<&Path> → Option<&Path>
        Type::Reference(TypeReference { elem, .. })
            if matches!(elem.as_ref(), Type::Path(tp) if path_ident_string(tp).as_deref() == Some("Path")) =>
        {
            let binding_name = Ident::new(
                &format!("{}_path", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name: Option<&std::path::Path> = req.#param_name
                        .as_deref()
                        .map(std::path::Path::new);
                },
                expr: quote! { #binding_name },
            })
        }

        // Option<&ElementId>
        Type::Reference(TypeReference { elem, .. })
            if matches!(elem.as_ref(), Type::Path(tp) if path_ident_string(tp).as_deref() == Some("ElementId")) =>
        {
            let binding_name = Ident::new(
                &format!("{}_parsed", param_name),
                param_name.span(),
            );
            Ok(ConversionResult {
                bindings: quote! {
                    let #binding_name: Option<sysml_id::ElementId> = req.#param_name
                        .as_ref()
                        .map(|s| sysml_id::ElementId::from_string(s.clone()));
                },
                expr: quote! { #binding_name.as_ref() },
            })
        }

        // Option<&[T]> (slice) → Vec<T_wire> on wire, .as_deref() for conversion
        Type::Reference(TypeReference { elem, .. })
            if matches!(elem.as_ref(), Type::Slice(_)) =>
        {
            Ok(ConversionResult {
                bindings: quote! {},
                expr: quote! { req.#param_name.as_deref() },
            })
        }

        // Option<usize>, Option<bool>, etc.
        Type::Path(tp) => {
            let name = path_ident_string(tp);
            match name.as_deref() {
                Some("usize") | Some("bool") | Some("u32") | Some("u64") | Some("i32")
                | Some("i64") | Some("f64") | Some("String") => Ok(ConversionResult {
                    bindings: quote! {},
                    expr: quote! { req.#param_name },
                }),
                _ => Err(format!(
                    "unsupported Option inner type for conversion: `Option<{}>`",
                    quote!(#inner)
                )),
            }
        }

        _ => Err(format!(
            "unsupported Option inner type for conversion: `Option<{}>`",
            quote!(#inner)
        )),
    }
}

// ── Helper functions ──────────────────────────────────────────────────────

/// Extracts the last segment ident of a type path as a string.
fn path_ident_string(tp: &TypePath) -> Option<String> {
    tp.path.segments.last().map(|s| s.ident.to_string())
}

/// Checks if a TypePath is `Option<_>`.
fn is_option_path(tp: &TypePath) -> bool {
    path_ident_string(tp).as_deref() == Some("Option")
}

/// Extracts the inner type T from `Option<T>`.
fn option_inner_type(tp: &TypePath) -> Option<&Type> {
    let last = tp.path.segments.last()?;
    if let PathArguments::AngleBracketed(ref args) = last.arguments {
        args.args.first().and_then(|arg| {
            if let GenericArgument::Type(ty) = arg {
                Some(ty)
            } else {
                None
            }
        })
    } else {
        None
    }
}

/// Extracts the first generic argument type from a path like `Vec<T>` or `HashSet<T>`.
fn first_generic_arg(tp: &TypePath) -> Option<&Type> {
    let last = tp.path.segments.last()?;
    if let PathArguments::AngleBracketed(ref args) = last.arguments {
        args.args.first().and_then(|arg| {
            if let GenericArgument::Type(ty) = arg {
                Some(ty)
            } else {
                None
            }
        })
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn parse_type(s: &str) -> Type {
        syn::parse_str::<Type>(s).unwrap()
    }

    #[test]
    fn wire_type_str_ref() {
        let ty = parse_type("&str");
        let wire = wire_type(&ty).unwrap();
        assert_eq!(wire.to_string(), "String");
    }

    #[test]
    fn wire_type_option_str() {
        let ty = parse_type("Option<&str>");
        let wire = wire_type(&ty).unwrap();
        assert_eq!(wire.to_string(), "Option < String >");
    }

    #[test]
    fn wire_type_slice() {
        let ty = parse_type("&[(String, String)]");
        let wire = wire_type(&ty).unwrap();
        assert_eq!(wire.to_string(), "Vec < (String , String) >");
    }

    #[test]
    fn wire_type_hashset() {
        let ty = parse_type("&HashSet<String>");
        let wire = wire_type(&ty).unwrap();
        assert_eq!(wire.to_string(), "Vec < String >");
    }

    #[test]
    fn type_string_basic() {
        assert_eq!(type_string(&parse_type("&str")), "string");
        assert_eq!(type_string(&parse_type("usize")), "usize");
        assert_eq!(type_string(&parse_type("bool")), "bool");
        assert_eq!(type_string(&parse_type("&ElementKind")), "ElementKind");
        assert_eq!(type_string(&parse_type("Option<&ElementKind>")), "ElementKind?");
        assert_eq!(type_string(&parse_type("Option<&str>")), "string?");
        assert_eq!(type_string(&parse_type("&[(String, String)]")), "[(string, string)]");
    }

    #[test]
    fn is_optional_check() {
        assert!(!is_optional(&parse_type("&str")));
        assert!(is_optional(&parse_type("Option<&str>")));
        assert!(is_optional(&parse_type("Option<usize>")));
    }
}
