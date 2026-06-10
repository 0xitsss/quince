use quote::ToTokens;
use syn::{parse_file, Item, Visibility};

fn sanitize_doc_text(text: &str) -> String {
    text.replace("](r#", "](").replace("crate::", "")
}

pub fn extract_module_doc(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut doc_lines: Vec<String> = Vec::new();
    let mut in_doc = false;

    for &line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("//!") {
            in_doc = true;
            doc_lines.push(trimmed.trim_start_matches("//!").trim_start().to_string());
        } else if trimmed.starts_with("//") || trimmed.starts_with("#![") {
            continue;
        } else if trimmed.is_empty() {
            if in_doc {
                doc_lines.push(String::new());
            }
            continue;
        } else if in_doc {
            break;
        }
    }

    while doc_lines.last().is_some_and(|l| l.is_empty()) {
        doc_lines.pop();
    }
    if doc_lines.is_empty() {
        return None;
    }
    let text = doc_lines.join("\n");
    Some(sanitize_doc_text(&text))
}

fn get_doc_comment(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(lit) = &nv.value {
                    if let syn::Lit::Str(s) = &lit.lit {
                        let val = s.value();
                        let trimmed = val.trim();
                        if !trimmed.is_empty() {
                            lines.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }
    sanitize_doc_text(&lines.join("\n"))
}

pub struct ExtractedItem {
    pub heading: String,
    pub code_block: String,
    pub doc_text: String,
}

pub enum ItemCategory {
    Structs,
    Enums,
    Unions,
    Traits,
    Functions,
    TypeAliases,
    Constants,
    Statics,
    Macros,
}

impl ItemCategory {
    pub fn heading(&self) -> &'static str {
        match self {
            ItemCategory::Structs => "Structs",
            ItemCategory::Enums => "Enums",
            ItemCategory::Unions => "Unions",
            ItemCategory::Traits => "Traits",
            ItemCategory::Functions => "Functions",
            ItemCategory::TypeAliases => "Type Aliases",
            ItemCategory::Constants => "Constants",
            ItemCategory::Statics => "Statics",
            ItemCategory::Macros => "Macros",
        }
    }
}

fn ty_to_string(ty: &syn::Type) -> String {
    ty.into_token_stream().to_string()
}

fn render_fields(fields: &syn::Fields) -> String {
    match fields {
        syn::Fields::Named(named) => {
            let mut s = String::new();
            for f in &named.named {
                let vis = if matches!(f.vis, Visibility::Public(_)) {
                    "pub "
                } else {
                    ""
                };
                let name = f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                let ty = ty_to_string(&f.ty);
                s.push_str(&format!("    {}{}: {},\n", vis, name, ty));
            }
            s
        }
        syn::Fields::Unnamed(unnamed) => {
            let mut s = String::new();
            for f in &unnamed.unnamed {
                let ty = ty_to_string(&f.ty);
                s.push_str(&format!("    {},\n", ty));
            }
            s
        }
        syn::Fields::Unit => String::new(),
    }
}

fn generics_to_string(g: &syn::Generics) -> String {
    if g.params.is_empty() {
        return String::new();
    }
    let params: Vec<String> = g
        .params
        .iter()
        .map(|p| p.into_token_stream().to_string())
        .collect();
    let mut s = format!("<{}>", params.join(", "));
    if let Some(wc) = &g.where_clause {
        let preds: Vec<String> = wc
            .predicates
            .iter()
            .map(|p| p.into_token_stream().to_string())
            .collect();
        s.push_str(&format!(" where {}", preds.join(", ")));
    }
    s
}

fn extract_struct(item: &syn::ItemStruct) -> Option<ExtractedItem> {
    let name = item.ident.to_string();
    let generics = generics_to_string(&item.generics);
    let heading = format!("`pub struct {}{}`", name, generics);
    let mut code = format!("pub struct {}{}", name, generics);
    match &item.fields {
        syn::Fields::Named(_) => {
            code.push_str(" {\n");
            code.push_str(&render_fields(&item.fields));
            code.push('}');
        }
        syn::Fields::Unnamed(_) => {
            code.push('(');
            let f = render_fields(&item.fields);
            code.push_str(f.trim());
            code.push_str(");");
        }
        syn::Fields::Unit => {
            code.push(';');
        }
    }
    code.push(';');
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

fn extract_fn(item: &syn::ItemFn) -> Option<ExtractedItem> {
    let heading = format!("`pub fn {}`", item.sig.ident);
    let code = format!("pub fn {}(...) {{ ... }}", item.sig.ident);
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

fn extract_enum(item: &syn::ItemEnum) -> Option<ExtractedItem> {
    let name = item.ident.to_string();
    let generics = generics_to_string(&item.generics);
    let heading = format!("`pub enum {}{}`", name, generics);
    let mut code = format!("pub enum {}{} {{\n", name, generics);
    for v in &item.variants {
        let vname = v.ident.to_string();
        let fields = render_fields(&v.fields);
        if fields.is_empty() {
            code.push_str(&format!("    {},\n", vname));
        } else {
            code.push_str(&format!("    {}({}),\n", vname, fields.trim()));
        }
    }
    code.push('}');
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

fn extract_trait(item: &syn::ItemTrait) -> Option<ExtractedItem> {
    let name = item.ident.to_string();
    let generics = generics_to_string(&item.generics);
    let heading = format!("`pub trait {}{}`", name, generics);
    let mut code = format!("pub trait {}{}", name, generics);
    for supertrait in &item.supertraits {
        code.push_str(&format!(": {}", supertrait.into_token_stream()));
    }
    code.push_str(" {\n");
    for titem in &item.items {
        if let syn::TraitItem::Fn(method) = titem {
            let sig = method.sig.clone().into_token_stream().to_string();
            let has_semicolon = if method.default.is_some() {
                " { ... }"
            } else {
                ";"
            };
            code.push_str(&format!("    {}{}\n", sig, has_semicolon));
        }
    }
    code.push('}');
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

fn extract_type_alias(item: &syn::ItemType) -> Option<ExtractedItem> {
    let name = item.ident.to_string();
    let ty = ty_to_string(&item.ty);
    let heading = format!("`pub type {}`", name);
    let code = format!("pub type {} = {};", name, ty);
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

fn extract_const(item: &syn::ItemConst) -> Option<ExtractedItem> {
    let name = item.ident.to_string();
    let ty = ty_to_string(&item.ty);
    let heading = format!("`pub const {}`", name);
    let code = format!("pub const {}: {} = ...;", name, ty);
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

fn extract_static(item: &syn::ItemStatic) -> Option<ExtractedItem> {
    let name = item.ident.to_string();
    let ty = ty_to_string(&item.ty);
    let heading = format!("`pub static {}`", name);
    let code = format!("pub static {}: {} = ...;", name, ty);
    let doc_text = get_doc_comment(&item.attrs);
    Some(ExtractedItem {
        heading,
        code_block: code,
        doc_text,
    })
}

pub fn extract_items(content: &str) -> Vec<(ItemCategory, Vec<ExtractedItem>)> {
    let file = match parse_file(content) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut unions = Vec::new();
    let mut traits = Vec::new();
    let mut fns = Vec::new();
    let mut type_aliases = Vec::new();
    let mut consts = Vec::new();
    let mut statics = Vec::new();
    let mut macros = Vec::new();

    for item in &file.items {
        match item {
            Item::Struct(s) if matches!(s.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_struct(s) {
                    structs.push(ei);
                }
            }
            Item::Enum(e) if matches!(e.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_enum(e) {
                    enums.push(ei);
                }
            }
            Item::Union(u) if matches!(u.vis, Visibility::Public(_)) => {
                let name = u.ident.to_string();
                let generics = generics_to_string(&u.generics);
                let doc_text = get_doc_comment(&u.attrs);
                unions.push(ExtractedItem {
                    heading: format!("`pub union {}{}`", name, generics),
                    code_block: format!("pub union {}{} {{ ... }}", name, generics),
                    doc_text,
                });
            }
            Item::Trait(t) if matches!(t.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_trait(t) {
                    traits.push(ei);
                }
            }
            Item::Fn(f) if matches!(f.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_fn(f) {
                    fns.push(ei);
                }
            }
            Item::Type(t) if matches!(t.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_type_alias(t) {
                    type_aliases.push(ei);
                }
            }
            Item::Const(c) if matches!(c.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_const(c) {
                    consts.push(ei);
                }
            }
            Item::Static(s) if matches!(s.vis, Visibility::Public(_)) => {
                if let Some(ei) = extract_static(s) {
                    statics.push(ei);
                }
            }
            Item::Macro(m) => {
                let doc = get_doc_comment(&m.attrs);
                if !doc.is_empty() {
                    macros.push(ExtractedItem {
                        heading: "`macro_rules!`".into(),
                        code_block: "macro_rules! { ... }".into(),
                        doc_text: doc,
                    });
                }
            }
            _ => {}
        }
    }

    let mut result = Vec::new();
    if !structs.is_empty() {
        result.push((ItemCategory::Structs, structs));
    }
    if !enums.is_empty() {
        result.push((ItemCategory::Enums, enums));
    }
    if !unions.is_empty() {
        result.push((ItemCategory::Unions, unions));
    }
    if !traits.is_empty() {
        result.push((ItemCategory::Traits, traits));
    }
    if !fns.is_empty() {
        result.push((ItemCategory::Functions, fns));
    }
    if !type_aliases.is_empty() {
        result.push((ItemCategory::TypeAliases, type_aliases));
    }
    if !consts.is_empty() {
        result.push((ItemCategory::Constants, consts));
    }
    if !statics.is_empty() {
        result.push((ItemCategory::Statics, statics));
    }
    if !macros.is_empty() {
        result.push((ItemCategory::Macros, macros));
    }
    result
}

pub fn render_items_section(category: &ItemCategory, items: &[ExtractedItem]) -> String {
    let mut out = format!("\n## {}\n\n", category.heading());
    for item in items {
        out.push_str(&format!("### {}\n\n", item.heading));
        out.push_str("```rust\n");
        out.push_str(&item.code_block);
        out.push_str("\n```\n\n");
        if !item.doc_text.is_empty() {
            out.push_str(&item.doc_text);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}
