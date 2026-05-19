//! Proc-macro derive for the `tonutils` TL-B runtime traits.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Expr, Fields, Ident, Lit, Result, Token, Type, parse_macro_input,
    spanned::Spanned,
};

#[proc_macro_derive(Tlb, attributes(tlb))]
pub fn derive_tlb(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_tlb(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(Contract, attributes(contract))]
pub fn derive_contract(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_contract(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_contract(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input.ident,
            "Contract derive supports named structs with exactly one data field",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "Contract derive supports named structs with exactly one data field",
        ));
    };
    if fields.named.len() != 1 {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "Contract derive requires exactly one named field: data",
        ));
    }
    let field = fields.named.first().expect("field count checked");
    let Some(field_name) = &field.ident else {
        return Err(syn::Error::new_spanned(
            field,
            "Contract derive requires a named data field",
        ));
    };
    if field_name != "data" {
        return Err(syn::Error::new_spanned(
            field_name,
            "Contract derive requires the field to be named data",
        ));
    }

    let config = contract_config(&input.attrs)?;
    let code_tokens = config.code_tokens()?;
    let workchain = config.workchain;
    let name = &input.ident;
    let data_ty = &field.ty;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::tonutils::contracts::ContractBlueprint for #name #ty_generics #where_clause {
            type Data = #data_ty;

            fn data(&self) -> &Self::Data {
                &self.data
            }

            fn code_boc(&self) -> ::std::borrow::Cow<'static, [u8]> {
                #code_tokens
            }

            fn workchain(&self) -> i8 {
                #workchain
            }
        }
    })
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

#[derive(Default)]
struct ContractConfig {
    code: Option<ContractCodeSource>,
    workchain: i8,
}

enum ContractCodeSource {
    Expr(Expr),
    Hex(Vec<u8>),
    File(syn::LitStr),
}

impl ContractConfig {
    fn code_tokens(&self) -> Result<proc_macro2::TokenStream> {
        let source = self.code.as_ref().ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "Contract derive requires one code source: code, code_hex, or code_file",
            )
        })?;
        Ok(match source {
            ContractCodeSource::Expr(expr) => {
                quote!(::std::borrow::Cow::Borrowed(#expr))
            }
            ContractCodeSource::Hex(bytes) => {
                let bytes = bytes.iter().copied();
                quote!(::std::borrow::Cow::Borrowed(&[#(#bytes),*]))
            }
            ContractCodeSource::File(path) => {
                quote!(::std::borrow::Cow::Borrowed(include_bytes!(#path)))
            }
        })
    }
}

fn contract_config(attrs: &[Attribute]) -> Result<ContractConfig> {
    let mut config = ContractConfig {
        code: None,
        workchain: 0,
    };
    let mut workchain_seen = false;

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("contract")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") {
                let value = meta.value()?;
                let expr: Expr = value.parse()?;
                set_contract_code(
                    &mut config,
                    ContractCodeSource::Expr(expr),
                    meta.path.span(),
                )?;
            } else if meta.path.is_ident("code_hex") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                let Lit::Str(lit) = lit else {
                    return Err(meta.error("code_hex must be a string literal"));
                };
                let bytes = parse_hex_bytes(&lit.value())
                    .map_err(|message| syn::Error::new(lit.span(), message))?;
                set_contract_code(
                    &mut config,
                    ContractCodeSource::Hex(bytes),
                    meta.path.span(),
                )?;
            } else if meta.path.is_ident("code_file") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                let Lit::Str(lit) = lit else {
                    return Err(meta.error("code_file must be a string literal"));
                };
                set_contract_code(&mut config, ContractCodeSource::File(lit), meta.path.span())?;
            } else if meta.path.is_ident("workchain") {
                if workchain_seen {
                    return Err(meta.error("workchain specified more than once"));
                }
                workchain_seen = true;
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                let Lit::Int(lit) = lit else {
                    return Err(meta.error("workchain must be an integer literal"));
                };
                config.workchain = lit.base10_parse::<i8>()?;
            } else {
                return Err(meta.error("unsupported contract attribute"));
            }
            Ok(())
        })?;
    }
    Ok(config)
}

fn set_contract_code(
    config: &mut ContractConfig,
    source: ContractCodeSource,
    span: proc_macro2::Span,
) -> Result<()> {
    if config.code.is_some() {
        return Err(syn::Error::new(
            span,
            "Contract derive accepts only one code source",
        ));
    }
    config.code = Some(source);
    Ok(())
}

fn parse_hex_bytes(raw: &str) -> std::result::Result<Vec<u8>, String> {
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw)
        .chars()
        .filter(|ch| *ch != '_' && !ch.is_ascii_whitespace())
        .collect::<String>();
    if hex.is_empty() {
        return Err("code_hex must not be empty".to_string());
    }
    if hex.len() % 2 != 0 {
        return Err("code_hex must contain an even number of hex digits".to_string());
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[index..index + 2], 16)
            .map_err(|_| "code_hex must contain only hexadecimal digits".to_string())?;
        bytes.push(byte);
    }
    Ok(bytes)
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
        expand_contract, inferred_unsigned_bits, is_float_primitive, normalize_tag_literal,
        parse_hex_bytes, requires_explicit_bits,
    };
    use syn::{DeriveInput, Type, parse_quote};

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

    #[test]
    fn contract_code_hex_accepts_prefixes_underscores_and_whitespace() {
        assert_eq!(
            parse_hex_bytes("0xb5ee_9c72").unwrap(),
            [0xb5, 0xee, 0x9c, 0x72]
        );
        assert_eq!(
            parse_hex_bytes("b5 ee 9c 72").unwrap(),
            [0xb5, 0xee, 0x9c, 0x72]
        );
    }

    #[test]
    fn contract_code_hex_rejects_empty_odd_and_invalid_values() {
        assert!(parse_hex_bytes("").is_err());
        assert!(parse_hex_bytes("abc").is_err());
        assert!(parse_hex_bytes("zz").is_err());
    }

    #[test]
    fn contract_derive_accepts_supported_code_sources() {
        let const_input: DeriveInput = parse_quote! {
            #[contract(code = CODE_BOC)]
            struct Wallet {
                data: WalletData,
            }
        };
        assert!(expand_contract(const_input).is_ok());

        let hex_input: DeriveInput = parse_quote! {
            #[contract(code_hex = "b5ee9c72010101010002000000", workchain = -1)]
            struct Wallet {
                data: WalletData,
            }
        };
        assert!(expand_contract(hex_input).is_ok());

        let file_input: DeriveInput = parse_quote! {
            #[contract(code_file = "wallet.code.boc")]
            struct Wallet {
                data: WalletData,
            }
        };
        assert!(expand_contract(file_input).is_ok());
    }

    #[test]
    fn contract_derive_rejects_ambiguous_or_unsupported_shapes() {
        let missing_data: DeriveInput = parse_quote! {
            #[contract(code = CODE_BOC)]
            struct Wallet {
                state: WalletData,
            }
        };
        assert!(expand_contract(missing_data).is_err());

        let extra_field: DeriveInput = parse_quote! {
            #[contract(code = CODE_BOC)]
            struct Wallet {
                data: WalletData,
                address: u32,
            }
        };
        assert!(expand_contract(extra_field).is_err());

        let unnamed: DeriveInput = parse_quote! {
            #[contract(code = CODE_BOC)]
            struct Wallet(WalletData);
        };
        assert!(expand_contract(unnamed).is_err());

        let unit: DeriveInput = parse_quote! {
            #[contract(code = CODE_BOC)]
            struct Wallet;
        };
        assert!(expand_contract(unit).is_err());

        let multiple_code_sources: DeriveInput = parse_quote! {
            #[contract(code = CODE_BOC, code_hex = "00")]
            struct Wallet {
                data: WalletData,
            }
        };
        assert!(expand_contract(multiple_code_sources).is_err());
    }
}
