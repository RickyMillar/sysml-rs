//! Parsing of `#[service_command(...)]` attributes and method signatures.
//!
//! Extracts structured metadata from the macro attribute arguments and the
//! annotated method's signature (parameters, types, doc comments).

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Attribute, FnArg, Ident, ImplItemFn, Lit, LitStr, Meta, MetaNameValue, Pat, PatType, Token,
    Type,
};

/// Parsed representation of the `#[service_command(...)]` attribute arguments.
#[derive(Debug)]
pub struct CommandAttrs {
    /// Dot-separated command name (e.g. `"sysml.find"`).
    pub name: String,
    /// Command category identifier (e.g. `Query`, `FileManagement`).
    pub category: Ident,
    /// Human-readable description.
    pub description: String,
    /// Return type description string (e.g. `"Vec<Element>"`).
    pub returns: String,
    /// Whether this command manages session state.
    pub stateful: bool,
    /// Whether this command is superseded and should be hidden from
    /// user-facing command listings (it stays dispatchable for existing
    /// callers). Prefer this flag over writing "[Deprecated: …]" into the
    /// `description`, which ships an internal note to end users.
    pub deprecated: bool,
}

impl Parse for CommandAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name: Option<String> = None;
        let mut category: Option<Ident> = None;
        let mut description: Option<String> = None;
        let mut returns: Option<String> = None;
        let mut stateful = false;
        let mut deprecated = false;

        let entries = Punctuated::<AttrEntry, Token![,]>::parse_terminated(input)?;

        for entry in entries {
            match entry.key.to_string().as_str() {
                "name" => {
                    name = Some(entry.value_as_str(&entry.key)?);
                }
                "category" => {
                    category = Some(entry.value_as_ident(&entry.key)?);
                }
                "description" => {
                    description = Some(entry.value_as_str(&entry.key)?);
                }
                "returns" => {
                    returns = Some(entry.value_as_str(&entry.key)?);
                }
                "stateful" => {
                    stateful = entry.value_as_bool(&entry.key)?;
                }
                "deprecated" => {
                    deprecated = entry.value_as_bool(&entry.key)?;
                }
                other => {
                    return Err(syn::Error::new(
                        entry.key.span(),
                        format!("unknown attribute `{}`", other),
                    ));
                }
            }
        }

        Ok(CommandAttrs {
            name: name.ok_or_else(|| {
                syn::Error::new(Span::call_site(), "missing required attribute `name`")
            })?,
            category: category.ok_or_else(|| {
                syn::Error::new(Span::call_site(), "missing required attribute `category`")
            })?,
            description: description.ok_or_else(|| {
                syn::Error::new(Span::call_site(), "missing required attribute `description`")
            })?,
            returns: returns.ok_or_else(|| {
                syn::Error::new(Span::call_site(), "missing required attribute `returns`")
            })?,
            stateful,
            deprecated,
        })
    }
}

/// A single `key = value` entry within the attribute.
struct AttrEntry {
    key: Ident,
    value: AttrValue,
}

enum AttrValue {
    Str(LitStr),
    Ident(Ident),
    Bool(syn::LitBool),
}

impl AttrEntry {
    fn value_as_str(&self, key: &Ident) -> syn::Result<String> {
        match &self.value {
            AttrValue::Str(lit) => Ok(lit.value()),
            _ => Err(syn::Error::new(
                key.span(),
                format!("`{}` expects a string literal", key),
            )),
        }
    }

    fn value_as_ident(&self, key: &Ident) -> syn::Result<Ident> {
        match &self.value {
            AttrValue::Ident(ident) => Ok(ident.clone()),
            _ => Err(syn::Error::new(
                key.span(),
                format!("`{}` expects an identifier", key),
            )),
        }
    }

    fn value_as_bool(&self, key: &Ident) -> syn::Result<bool> {
        match &self.value {
            AttrValue::Bool(lit) => Ok(lit.value()),
            AttrValue::Ident(ident) if ident == "true" => Ok(true),
            AttrValue::Ident(ident) if ident == "false" => Ok(false),
            _ => Err(syn::Error::new(
                key.span(),
                format!("`{}` expects a boolean", key),
            )),
        }
    }
}

impl Parse for AttrEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        let _eq: Token![=] = input.parse()?;

        let value = if input.peek(LitStr) {
            AttrValue::Str(input.parse()?)
        } else if input.peek(syn::LitBool) {
            AttrValue::Bool(input.parse()?)
        } else {
            AttrValue::Ident(input.parse()?)
        };

        Ok(AttrEntry { key, value })
    }
}

/// Parsed information about a single method parameter.
#[derive(Debug)]
pub struct ParamInfo {
    /// Parameter name.
    pub name: Ident,
    /// Original Rust type from the method signature.
    pub ty: Type,
    /// Documentation string (from `#[doc = "..."]` or fallback to param name).
    pub doc: String,
}

/// Parsed information about the annotated method.
#[derive(Debug)]
pub struct MethodInfo {
    /// Method name identifier.
    pub name: Ident,
    /// Parameters (excluding `&self`).
    pub params: Vec<ParamInfo>,
    /// The full return type of the method (retained for future use).
    pub _return_type: Type,
    /// Whether the return type is `Result<T, _>`.
    pub returns_result: bool,
    /// The inner `T` from `Result<T, E>`, or the return type itself if not Result.
    pub response_type: Type,
}

impl MethodInfo {
    /// Extract method information from a parsed `ImplItemFn`.
    pub fn from_method(method: &ImplItemFn) -> syn::Result<Self> {
        let name = method.sig.ident.clone();

        // Collect parameters, skipping &self
        let params: Vec<ParamInfo> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Receiver(_) => None,
                FnArg::Typed(pat_type) => Some(parse_param(pat_type)),
            })
            .collect::<syn::Result<Vec<_>>>()?;

        // Parse return type
        let return_type = match &method.sig.output {
            // Infallible: parsing literal "()"
            #[allow(clippy::unwrap_used)]
            syn::ReturnType::Default => syn::parse_str::<Type>("()").unwrap(),
            syn::ReturnType::Type(_, ty) => *ty.clone(),
        };

        let (returns_result, response_type) = extract_response_type(&return_type);

        Ok(MethodInfo {
            name,
            params,
            _return_type: return_type,
            returns_result,
            response_type,
        })
    }
}

/// Parse a single typed parameter, extracting its doc comment.
fn parse_param(pat_type: &PatType) -> syn::Result<ParamInfo> {
    let name = match pat_type.pat.as_ref() {
        Pat::Ident(pat_ident) => pat_ident.ident.clone(),
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "expected a simple identifier pattern for parameter",
            ));
        }
    };

    let doc = extract_doc_from_attrs(&pat_type.attrs).unwrap_or_else(|| name.to_string());

    Ok(ParamInfo {
        name,
        ty: *pat_type.ty.clone(),
        doc,
    })
}

/// Extract the doc string from `#[doc = "..."]` attributes.
fn extract_doc_from_attrs(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            }) = &attr.meta
            {
                return Some(lit_str.value().trim().to_owned());
            }
        }
    }
    None
}

/// If the return type is `Result<T, E>`, extract `T`. Otherwise return the type as-is.
fn extract_response_type(ty: &Type) -> (bool, Type) {
    if let Type::Path(type_path) = ty {
        if let Some(last_segment) = type_path.path.segments.last() {
            if last_segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        return (true, inner.clone());
                    }
                }
            }
        }
    }
    (false, ty.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_attrs() {
        let tokens: proc_macro2::TokenStream = quote::quote! {
            name = "sysml.find",
            category = Query,
            description = "Find elements by name pattern",
            returns = "Vec<Element>"
        };
        let attrs: CommandAttrs = syn::parse2(tokens).unwrap();
        assert_eq!(attrs.name, "sysml.find");
        assert_eq!(attrs.category, "Query");
        assert_eq!(attrs.description, "Find elements by name pattern");
        assert_eq!(attrs.returns, "Vec<Element>");
        assert!(!attrs.stateful);
        assert!(!attrs.deprecated);
    }

    #[test]
    fn parse_command_attrs_with_stateful() {
        let tokens: proc_macro2::TokenStream = quote::quote! {
            name = "sysml.simulate.start",
            category = Execution,
            description = "Start a simulation session",
            returns = "SessionId",
            stateful = true
        };
        let attrs: CommandAttrs = syn::parse2(tokens).unwrap();
        assert!(attrs.stateful);
        assert!(!attrs.deprecated);
    }

    #[test]
    fn parse_command_attrs_with_deprecated() {
        let tokens: proc_macro2::TokenStream = quote::quote! {
            name = "sysml.orchestrate.start",
            category = Execution,
            description = "Start a multi-subsystem orchestrator session from the model",
            returns = "(session_key: string, ExecutionSnapshot)",
            stateful = true,
            deprecated = true
        };
        let attrs: CommandAttrs = syn::parse2(tokens).unwrap();
        assert!(attrs.deprecated);
        assert!(
            !attrs.description.contains("Deprecated"),
            "the deprecation belongs in the flag, not in the user-visible description"
        );
    }

    #[test]
    fn extract_result_response_type() {
        let ty: Type = syn::parse_str("Result<Vec<Element>, ServiceError>").unwrap();
        let (is_result, response) = extract_response_type(&ty);
        assert!(is_result);
        assert_eq!(quote::quote!(#response).to_string(), "Vec < Element >");
    }

    #[test]
    fn extract_non_result_response_type() {
        let ty: Type = syn::parse_str("Vec<String>").unwrap();
        let (is_result, response) = extract_response_type(&ty);
        assert!(!is_result);
        assert_eq!(quote::quote!(#response).to_string(), "Vec < String >");
    }
}
