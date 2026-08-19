//! Procedural derives for the `tonutils-tlb` TL-B runtime traits.
//!
//! Most users should depend on [`tonutils_tlb`](https://docs.rs/tonutils-tlb)
//! and use its re-exported derive workflow. This crate is the implementation
//! package for the `Tlb` derive and is normally pulled in transitively.
//!
//! The crate is a compile-time helper with no runtime or network behavior.
//! Depend on [`tonutils_tlb`](https://docs.rs/tonutils-tlb) for the public
//! TL-B traits and feature-gated derive workflow.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Fields, Ident, Lit, Result, Token, Type, parse_macro_input,
};

#[proc_macro_derive(Tlb, attributes(tlb))]
pub fn derive_tlb(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_tlb(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_tlb(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    match input.data {
        Data::Struct(data) => expand_struct(&input.ident, &input.attrs, &data.fields),
        Data::Enum(data) => {
            expand_enum(&input.ident, &data.variants.into_iter().collect::<Vec<_>>())
        }
        Data::Union(_) => Err(syn::Error::new_spanned(
            input.ident,
            "TL-B derive does not support unions",
        )),
    }
}

fn expand_struct(
    name: &Ident,
    attrs: &[Attribute],
    fields: &Fields,
) -> Result<proc_macro2::TokenStream> {
    let tag = tlb_tag(attrs)?;
    let field_specs = field_specs(fields)?;
    let store_fields = field_specs.iter().map(|field| {
        let access = &field.access;
        field.store_tokens(&quote!(&self.#access))
    });
    let load_fields = field_specs.iter().map(|field| {
        let binding = &field.binding;
        let load = field.load_tokens();
        quote!(let #binding = #load;)
    });
    let construct = construct_struct(name, fields, &field_specs);
    let store_tag = tag
        .as_deref()
        .map(|tag| quote!(::tonutils_tlb::store_tag(builder, #tag)?;))
        .unwrap_or_default();
    let load_tag = tag
        .as_deref()
        .map(|tag| quote!(::tonutils_tlb::expect_tag(slice, stringify!(#name), #tag)?;))
        .unwrap_or_default();

    Ok(quote! {
        impl ::tonutils_tlb::TlbSerialize for #name {
            fn store_tlb(&self, builder: &mut ::tonutils_tvm::Builder) -> ::tonutils_tlb::Result<()> {
                #store_tag
                #(#store_fields)*
                Ok(())
            }
        }

        impl ::tonutils_tlb::TlbDeserialize for #name {
            fn load_tlb(slice: &mut ::tonutils_tvm::Slice) -> ::tonutils_tlb::Result<Self> {
                #load_tag
                #(#load_fields)*
                Ok(#construct)
            }
        }
    })
}

fn expand_enum(name: &Ident, variants: &[syn::Variant]) -> Result<proc_macro2::TokenStream> {
    let mut store_arms = Vec::new();
    let mut load_arms = Vec::new();
    let mut expected_tags = Vec::new();
    let max_tag_len = variants
        .iter()
        .filter_map(|variant| tlb_tag(&variant.attrs).ok().flatten())
        .map(|tag| tag.len())
        .max()
        .unwrap_or(0);

    for variant in variants {
        let variant_name = &variant.ident;
        let tag = tlb_tag(&variant.attrs)?.ok_or_else(|| {
            syn::Error::new_spanned(
                variant_name,
                "TL-B enum variants require #[tlb(tag = \"...\")]",
            )
        })?;
        expected_tags.push(tag.clone());
        let specs = field_specs(&variant.fields)?;
        let bindings = specs.iter().map(|field| &field.binding).collect::<Vec<_>>();
        let pattern = match &variant.fields {
            Fields::Named(_) => quote!(#name::#variant_name { #(#bindings),* }),
            Fields::Unnamed(_) => quote!(#name::#variant_name(#(#bindings),*)),
            Fields::Unit => quote!(#name::#variant_name),
        };
        let store_fields = specs.iter().map(|field| {
            let binding = &field.binding;
            field.store_tokens(&quote!(#binding))
        });
        store_arms.push(quote! {
            #pattern => {
                ::tonutils_tlb::store_tag(builder, #tag)?;
                #(#store_fields)*
            }
        });

        let load_fields = specs.iter().map(|field| {
            let binding = &field.binding;
            let load = field.load_tokens();
            quote!(let #binding = #load;)
        });
        let construct = construct_variant(name, variant_name, &variant.fields, &specs);
        load_arms.push(quote! {
            #tag => {
                #(#load_fields)*
                return Ok(#construct);
            }
        });
    }
    let expected = expected_tags.join("|");

    Ok(quote! {
        impl ::tonutils_tlb::TlbSerialize for #name {
            fn store_tlb(&self, builder: &mut ::tonutils_tvm::Builder) -> ::tonutils_tlb::Result<()> {
                match self {
                    #(#store_arms),*
                }
                Ok(())
            }
        }

        impl ::tonutils_tlb::TlbDeserialize for #name {
            fn load_tlb(slice: &mut ::tonutils_tvm::Slice) -> ::tonutils_tlb::Result<Self> {
                let mut actual = String::new();
                while actual.len() < #max_tag_len {
                    let bit = slice.load_bit()?;
                    actual.push(if bit { '1' } else { '0' });
                    match actual.as_str() {
                        #(#load_arms)*
                        _ => {}
                    }
                }
                Err(::tonutils_tlb::TlbError::TagMismatch {
                    constructor: stringify!(#name),
                    expected_bits: #expected,
                    actual_bits: actual,
                })
            }
        }
    })
}

#[derive(Clone)]
struct FieldSpec {
    binding: Ident,
    access: proc_macro2::TokenStream,
    ty: Type,
    bits: Option<usize>,
    referenced: bool,
}

impl FieldSpec {
    fn store_tokens(&self, value: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        if self.referenced {
            return quote!(::tonutils_tlb::store_ref_tlb(builder, #value)?;);
        }
        if let Some(bits) = self.bits {
            return quote!(::tonutils_tlb::StoreBits::<#bits>::store_bits_tlb(#value, builder)?;);
        }
        quote!(::tonutils_tlb::TlbSerialize::store_tlb(#value, builder)?;)
    }

    fn load_tokens(&self) -> proc_macro2::TokenStream {
        let ty = &self.ty;
        if self.referenced {
            return quote!(::tonutils_tlb::load_ref_tlb::<#ty>(slice, stringify!(#ty))?);
        }
        if let Some(bits) = self.bits {
            return quote!(<#ty as ::tonutils_tlb::LoadBits<#bits>>::load_bits_tlb(slice)?);
        }
        quote!(<#ty as ::tonutils_tlb::TlbDeserialize>::load_tlb(slice)?)
    }
}

fn field_specs(fields: &Fields) -> Result<Vec<FieldSpec>> {
    fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let binding = field
                .ident
                .clone()
                .unwrap_or_else(|| format_ident!("field_{index}"));
            let access = field.ident.as_ref().map_or_else(
                || {
                    let index = syn::Index::from(index);
                    quote!(#index)
                },
                |ident| quote!(#ident),
            );
            Ok(FieldSpec {
                binding,
                access,
                ty: field.ty.clone(),
                bits: field_bits(field)?,
                referenced: tlb_flag(&field.attrs, "reference")? || tlb_flag(&field.attrs, "ref")?,
            })
        })
        .collect()
}

fn field_bits(field: &syn::Field) -> Result<Option<usize>> {
    if is_float_primitive(&field.ty) {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "float primitive TL-B fields are not supported by the runtime",
        ));
    }
    if let Some(bits) = tlb_bits(&field.attrs)? {
        return Ok(Some(bits));
    }
    if let Some(bits) = inferred_unsigned_bits(&field.ty) {
        return Ok(Some(bits));
    }
    if requires_explicit_bits(&field.ty) {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "signed integer and float TL-B fields require #[tlb(bits = N)]",
        ));
    }
    Ok(None)
}

fn inferred_unsigned_bits(ty: &Type) -> Option<usize> {
    match primitive_type_ident(ty)?.as_str() {
        "u8" => Some(8),
        "u16" => Some(16),
        "u32" => Some(32),
        "u64" => Some(64),
        "u128" => Some(128),
        _ => None,
    }
}

fn requires_explicit_bits(ty: &Type) -> bool {
    matches!(
        primitive_type_ident(ty).as_deref(),
        Some("i8" | "i16" | "i32" | "i64" | "i128" | "isize")
    )
}

fn is_float_primitive(ty: &Type) -> bool {
    matches!(primitive_type_ident(ty).as_deref(), Some("f32" | "f64"))
}

fn primitive_type_ident(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    Some(path.path.segments.first()?.ident.to_string())
}

fn construct_struct(
    name: &Ident,
    fields: &Fields,
    specs: &[FieldSpec],
) -> proc_macro2::TokenStream {
    let bindings = specs.iter().map(|field| &field.binding);
    match fields {
        Fields::Named(_) => quote!(#name { #(#bindings),* }),
        Fields::Unnamed(_) => quote!(#name(#(#bindings),*)),
        Fields::Unit => quote!(#name),
    }
}

fn construct_variant(
    name: &Ident,
    variant: &Ident,
    fields: &Fields,
    specs: &[FieldSpec],
) -> proc_macro2::TokenStream {
    let bindings = specs.iter().map(|field| &field.binding);
    match fields {
        Fields::Named(_) => quote!(#name::#variant { #(#bindings),* }),
        Fields::Unnamed(_) => quote!(#name::#variant(#(#bindings),*)),
        Fields::Unit => quote!(#name::#variant),
    }
}

fn normalize_tag_literal(raw: &str) -> std::result::Result<String, &'static str> {
    let value = raw.replace('_', "");
    if value.is_empty() {
        return Err("TL-B tag cannot be empty");
    }
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix('#')) {
        if hex.is_empty() || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("TL-B hex tag is invalid");
        }
        return Ok(hex
            .bytes()
            .flat_map(|byte| {
                let digit = byte.to_ascii_lowercase();
                let value = if digit.is_ascii_digit() {
                    digit - b'0'
                } else {
                    digit - b'a' + 10
                };
                format!("{value:04b}").chars().collect::<Vec<_>>()
            })
            .collect());
    }
    let binary = value.strip_prefix("0b").unwrap_or(&value);
    if binary.bytes().all(|byte| matches!(byte, b'0' | b'1')) {
        Ok(binary.to_owned())
    } else {
        Err("TL-B tag must be binary or hexadecimal")
    }
}

fn tlb_tag(attrs: &[Attribute]) -> Result<Option<String>> {
    let mut tag = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("tlb")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("tag") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                match lit {
                    Lit::Str(lit) => {
                        tag = Some(
                            normalize_tag_literal(&lit.value())
                                .map_err(|message| syn::Error::new(lit.span(), message))?,
                        );
                    }
                    _ => return Err(meta.error("tag must be a string literal")),
                }
            }
            Ok(())
        })?;
    }
    Ok(tag)
}

fn tlb_bits(attrs: &[Attribute]) -> Result<Option<usize>> {
    let mut bits = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("tlb")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("bits") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Int(lit) = lit {
                    bits = Some(lit.base10_parse()?);
                    return Ok(());
                }
                return Err(meta.error("bits must be an integer literal"));
            }
            Ok(())
        })?;
    }
    Ok(bits)
}

fn tlb_flag(attrs: &[Attribute], name: &str) -> Result<bool> {
    let mut found = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("tlb")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(name) {
                found = true;
            } else if meta.input.peek(Token![=]) {
                let _ = meta.value()?.parse::<Lit>()?;
            }
            Ok(())
        })?;
    }
    Ok(found)
}
