//! Proc-macro derive for the `tonutils` TL-B runtime traits.

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
        field.store_tokens(quote!(&self.#access))
    });
    let load_fields = field_specs.iter().map(|field| {
        let binding = &field.binding;
        let load = field.load_tokens();
        quote!(let #binding = #load;)
    });
    let construct = construct_struct(name, fields, &field_specs);
    let store_tag = tag
        .as_deref()
        .map(|tag| quote!(::tonutils::tlb::store_tag(builder, #tag)?;))
        .unwrap_or_default();
    let load_tag = tag
        .as_deref()
        .map(|tag| quote!(::tonutils::tlb::expect_tag(slice, stringify!(#name), #tag)?;))
        .unwrap_or_default();

    Ok(quote! {
        impl ::tonutils::tlb::TlbSerialize for #name {
            fn store_tlb(&self, builder: &mut ::tonutils::tvm::Builder) -> ::tonutils::tlb::Result<()> {
                #store_tag
                #(#store_fields)*
                Ok(())
            }
        }

        impl ::tonutils::tlb::TlbDeserialize for #name {
            fn load_tlb(slice: &mut ::tonutils::tvm::Slice) -> ::tonutils::tlb::Result<Self> {
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
            field.store_tokens(quote!(#binding))
        });
        store_arms.push(quote! {
            #pattern => {
                ::tonutils::tlb::store_tag(builder, #tag)?;
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
        impl ::tonutils::tlb::TlbSerialize for #name {
            fn store_tlb(&self, builder: &mut ::tonutils::tvm::Builder) -> ::tonutils::tlb::Result<()> {
                match self {
                    #(#store_arms),*
                }
                Ok(())
            }
        }

        impl ::tonutils::tlb::TlbDeserialize for #name {
            fn load_tlb(slice: &mut ::tonutils::tvm::Slice) -> ::tonutils::tlb::Result<Self> {
                let mut actual = String::new();
                while actual.len() < #max_tag_len {
                    let bit = slice.load_bit()?;
                    actual.push(if bit { '1' } else { '0' });
                    match actual.as_str() {
                        #(#load_arms)*
                        _ => {}
                    }
                }
                Err(::tonutils::tlb::TlbError::TagMismatch {
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
    fn store_tokens(&self, value: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        if self.referenced {
            return quote!(::tonutils::tlb::store_ref_tlb(builder, #value)?;);
        }
        if let Some(bits) = self.bits {
            return quote!(::tonutils::tlb::StoreBits::<#bits>::store_bits_tlb(#value, builder)?;);
        }
        quote!(::tonutils::tlb::TlbSerialize::store_tlb(#value, builder)?;)
    }

    fn load_tokens(&self) -> proc_macro2::TokenStream {
        let ty = &self.ty;
        if self.referenced {
            return quote!(::tonutils::tlb::load_ref_tlb::<#ty>(slice, stringify!(#ty))?);
        }
        if let Some(bits) = self.bits {
            return quote!(<#ty as ::tonutils::tlb::LoadBits<#bits>>::load_bits_tlb(slice)?);
        }
        quote!(<#ty as ::tonutils::tlb::TlbDeserialize>::load_tlb(slice)?)
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
            let access = field
                .ident
                .as_ref()
                .map(|ident| quote!(#ident))
                .unwrap_or_else(|| {
                    let index = syn::Index::from(index);
                    quote!(#index)
                });
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
                        )
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

fn normalize_tag_literal(raw: &str) -> std::result::Result<String, String> {
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        return hex_tag_to_bits(hex);
    }
    if let Some(hex) = raw.strip_prefix('#') {
        return hex_tag_to_bits(hex);
    }
    if let Some(bits) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
        return binary_tag_to_bits(bits);
    }
    binary_tag_to_bits(raw)
}

fn binary_tag_to_bits(raw: &str) -> std::result::Result<String, String> {
    let bits = raw.chars().filter(|ch| *ch != '_').collect::<String>();
    if bits.is_empty() {
        return Err("tag must not be empty".to_string());
    }
    if bits.chars().all(|ch| matches!(ch, '0' | '1')) {
        Ok(bits)
    } else {
        Err("binary tag must contain only 0, 1, or _; use 0x... or #... for hex tags".to_string())
    }
}

fn hex_tag_to_bits(raw: &str) -> std::result::Result<String, String> {
    let hex = raw.chars().filter(|ch| *ch != '_').collect::<String>();
    if hex.is_empty() {
        return Err("hex tag must not be empty".to_string());
    }

    let mut bits = String::with_capacity(hex.len() * 4);
    for ch in hex.chars() {
        let value = ch
            .to_digit(16)
            .ok_or_else(|| "hex tag must contain only hexadecimal digits or _".to_string())?;
        bits.push(if value & 0b1000 != 0 { '1' } else { '0' });
        bits.push(if value & 0b0100 != 0 { '1' } else { '0' });
        bits.push(if value & 0b0010 != 0 { '1' } else { '0' });
        bits.push(if value & 0b0001 != 0 { '1' } else { '0' });
    }
    Ok(bits)
}

#[cfg(test)]
mod tests {
    use super::{
        inferred_unsigned_bits, is_float_primitive, normalize_tag_literal, requires_explicit_bits,
    };
    use syn::{Type, parse_quote};

    #[test]
    fn tag_literals_accept_binary_and_hex_forms() {
        assert_eq!(normalize_tag_literal("101").unwrap(), "101");
        assert_eq!(normalize_tag_literal("0b10_01").unwrap(), "1001");
        assert_eq!(
            normalize_tag_literal("0x0f8a_7ea5").unwrap(),
            "00001111100010100111111010100101"
        );
        assert_eq!(normalize_tag_literal("#A5").unwrap(), "10100101");
    }

    #[test]
    fn tag_literals_reject_invalid_forms() {
        assert!(normalize_tag_literal("").is_err());
        assert!(normalize_tag_literal("102").is_err());
        assert!(normalize_tag_literal("0x").is_err());
        assert!(normalize_tag_literal("0xzz").is_err());
    }

    #[test]
    fn unsigned_primitive_bits_are_inferred() {
        let cases = [
            (parse_quote!(u8), Some(8)),
            (parse_quote!(u16), Some(16)),
            (parse_quote!(u32), Some(32)),
            (parse_quote!(u64), Some(64)),
            (parse_quote!(u128), Some(128)),
            (parse_quote!(usize), None),
            (parse_quote!(Grams), None),
        ];

        for (ty, expected) in cases {
            assert_eq!(inferred_unsigned_bits(&ty), expected);
        }
    }

    #[test]
    fn signed_and_float_primitives_require_explicit_bits() {
        let required: [Type; 5] = [
            parse_quote!(i8),
            parse_quote!(i16),
            parse_quote!(i32),
            parse_quote!(i64),
            parse_quote!(i128),
        ];
        for ty in required {
            assert!(requires_explicit_bits(&ty));
        }

        assert!(!requires_explicit_bits(&parse_quote!(u64)));
        assert!(!requires_explicit_bits(&parse_quote!(Grams)));
    }

    #[test]
    fn float_primitives_are_rejected() {
        assert!(is_float_primitive(&parse_quote!(f32)));
        assert!(is_float_primitive(&parse_quote!(f64)));
        assert!(!is_float_primitive(&parse_quote!(i64)));
    }
}
