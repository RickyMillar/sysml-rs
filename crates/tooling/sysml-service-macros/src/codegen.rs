//! Code generation for `#[service_command]`.
//!
//! Produces four items from each annotated method:
//! 1. A request struct (e.g. `FindRequest`)
//! 2. A `CommandMeta` constant (e.g. `FIND_META`)
//! 3. A zero-sized command struct with `ServiceCommand` impl (e.g. `FindCommand`)
//! 4. An `inventory::submit!` registration block

use convert_case::{Case, Casing};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};

use crate::parse::{CommandAttrs, MethodInfo};
use crate::type_mapping;

/// Generate all companion items for a `#[service_command]`-annotated method.
pub fn generate(attrs: &CommandAttrs, method: &MethodInfo) -> Result<TokenStream, syn::Error> {
    let request_struct = generate_request_struct(method)?;
    let meta_const = generate_meta_const(attrs, method)?;
    let command_struct = generate_command_struct(attrs, method)?;
    let inventory_reg = generate_inventory_registration(method)?;

    Ok(quote! {
        #request_struct
        #meta_const
        #command_struct
        #inventory_reg
    })
}

/// Generate the request struct (e.g. `FindRequest`).
///
/// Each method parameter becomes a field with its wire type.
/// Optional parameters get `#[serde(default, skip_serializing_if = "Option::is_none")]`.
fn generate_request_struct(method: &MethodInfo) -> Result<TokenStream, syn::Error> {
    let struct_name = request_struct_name(&method.name);

    let fields: Vec<TokenStream> = method
        .params
        .iter()
        .map(|p| {
            let field_name = &p.name;
            let wire_ty = type_mapping::wire_type(&p.ty).map_err(|msg| {
                syn::Error::new_spanned(&p.ty, msg)
            })?;

            let serde_attrs = if type_mapping::is_optional(&p.ty) {
                quote! {
                    #[serde(default, skip_serializing_if = "Option::is_none")]
                }
            } else {
                quote! {}
            };

            Ok(quote! {
                #serde_attrs
                pub #field_name: #wire_ty,
            })
        })
        .collect::<Result<Vec<_>, syn::Error>>()?;

    Ok(quote! {
        #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
        pub struct #struct_name {
            #(#fields)*
        }
    })
}

/// Generate the `CommandMeta` constant (e.g. `FIND_META`).
fn generate_meta_const(
    attrs: &CommandAttrs,
    method: &MethodInfo,
) -> Result<TokenStream, syn::Error> {
    let const_name = meta_const_name(&method.name);
    let cmd_name = &attrs.name;
    let category = &attrs.category;
    let description = &attrs.description;
    let returns = &attrs.returns;
    let stateful = attrs.stateful;
    let deprecated = attrs.deprecated;

    let param_metas: Vec<TokenStream> = method
        .params
        .iter()
        .map(|p| {
            let name_str = p.name.to_string();
            let ty_str = type_mapping::type_string(&p.ty);
            let required = !type_mapping::is_optional(&p.ty);
            let doc = &p.doc;

            quote! {
                crate::command_meta::ParamMeta {
                    name: #name_str,
                    ty: #ty_str,
                    required: #required,
                    description: #doc,
                }
            }
        })
        .collect();

    Ok(quote! {
        pub const #const_name: crate::command_meta::CommandMeta = crate::command_meta::CommandMeta {
            name: #cmd_name,
            category: crate::command_meta::CommandCategory::#category,
            description: #description,
            params: &[
                #(#param_metas),*
            ],
            returns: #returns,
            stateful: #stateful,
            deprecated: #deprecated,
        };
    })
}

/// Generate the zero-sized command struct and its `ServiceCommand` impl.
fn generate_command_struct(
    _attrs: &CommandAttrs,
    method: &MethodInfo,
) -> Result<TokenStream, syn::Error> {
    let command_struct_name = command_struct_ident(&method.name);
    let meta_const = meta_const_name(&method.name);
    let request_struct = request_struct_name(&method.name);
    let response_type = &method.response_type;
    let method_ident = &method.name;

    // Collect all bindings and argument expressions
    let mut all_bindings = Vec::new();
    let mut arg_exprs = Vec::new();

    for param in &method.params {
        let conversion = type_mapping::conversion_expr(&param.name, &param.ty).map_err(|msg| {
            syn::Error::new_spanned(&param.ty, msg)
        })?;
        all_bindings.push(conversion.bindings);
        arg_exprs.push(conversion.expr);
    }

    // If the method returns Result, call directly. Otherwise wrap in Ok().
    let call_expr = if method.returns_result {
        quote! {
            #(#all_bindings)*
            service.#method_ident(#(#arg_exprs),*)
        }
    } else {
        quote! {
            #(#all_bindings)*
            Ok(service.#method_ident(#(#arg_exprs),*))
        }
    };

    Ok(quote! {
        pub struct #command_struct_name;

        impl crate::command_trait::ServiceCommand for #command_struct_name {
            const META: crate::command_meta::CommandMeta = #meta_const;
            type Request = #request_struct;
            type Response = #response_type;

            fn execute(
                service: &SysmlService,
                req: Self::Request,
            ) -> Result<Self::Response, crate::ServiceError> {
                #call_expr
            }
        }
    })
}

/// Generate the `inventory::submit!` registration block.
fn generate_inventory_registration(method: &MethodInfo) -> Result<TokenStream, syn::Error> {
    let meta_const = meta_const_name(&method.name);
    let request_struct = request_struct_name(&method.name);
    let command_struct = command_struct_ident(&method.name);

    Ok(quote! {
        inventory::submit! {
            crate::command_trait::CommandRegistration {
                meta: &#meta_const,
                handler: |service: &SysmlService, body: serde_json::Value| -> Result<serde_json::Value, crate::ServiceError> {
                    let req: #request_struct = serde_json::from_value(body)
                        .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
                    let result = <#command_struct as crate::command_trait::ServiceCommand>::execute(service, req)?;
                    serde_json::to_value(&result)
                        .map_err(|e| crate::ServiceError::Internal(e.to_string()))
                },
            }
        }
    })
}

// ── Naming helpers ────────────────────────────────────────────────────────

/// `find` → `FindRequest`
fn request_struct_name(method_name: &Ident) -> Ident {
    let pascal = method_name.to_string().to_case(Case::Pascal);
    format_ident!("{}Request", pascal)
}

/// `find` → `FIND_META`
fn meta_const_name(method_name: &Ident) -> Ident {
    let screaming = method_name.to_string().to_case(Case::ScreamingSnake);
    Ident::new(&format!("{}_META", screaming), Span::call_site())
}

/// `find` → `FindCommand`
fn command_struct_ident(method_name: &Ident) -> Ident {
    let pascal = method_name.to_string().to_case(Case::Pascal);
    format_ident!("{}Command", pascal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_conventions() {
        let ident = Ident::new("find_elements", Span::call_site());
        assert_eq!(request_struct_name(&ident).to_string(), "FindElementsRequest");
        assert_eq!(meta_const_name(&ident).to_string(), "FIND_ELEMENTS_META");
        assert_eq!(command_struct_ident(&ident).to_string(), "FindElementsCommand");
    }

    #[test]
    fn naming_single_word() {
        let ident = Ident::new("find", Span::call_site());
        assert_eq!(request_struct_name(&ident).to_string(), "FindRequest");
        assert_eq!(meta_const_name(&ident).to_string(), "FIND_META");
        assert_eq!(command_struct_ident(&ident).to_string(), "FindCommand");
    }
}
