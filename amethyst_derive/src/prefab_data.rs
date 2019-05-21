use proc_macro2::{Literal, TokenStream};
use quote::quote;
use syn::{
    Attribute, Data, DataEnum, DataStruct, DeriveInput, Fields, Generics, Ident, Meta, NestedMeta,
    Type,
};

pub fn impl_prefab_data(ast: &DeriveInput) -> TokenStream {
    if have_component_attribute(&ast.attrs[..]) {
        impl_prefab_data_component(ast)
    } else {
        impl_prefab_data_aggregate(ast)
    }
}

fn impl_prefab_data_component(ast: &DeriveInput) -> TokenStream {
    let base = &ast.ident;
    let (_, ty_generics, where_clause) = ast.generics.split_for_impl();
    let lf_tokens = gen_def_lt_tokens(&ast.generics);
    let ty_tokens = gen_def_ty_params(&ast.generics);

    quote! {
        impl<'pfd, #lf_tokens #ty_tokens> PrefabData<'pfd> for #base #ty_generics #where_clause {
            type SystemData = WriteStorage<'pfd, #base #ty_generics>;
            type Result = ();

            fn add_to_entity(&self,
                             entity: Entity,
                             system_data: &mut Self::SystemData,
                             _: &[Entity],
                             _: &[Entity]) -> ::std::result::Result<(), Error> {
                system_data.insert(entity, self.clone()).map(|_| ())?;
                Ok(())
            }
        }
    }
}

fn prepare_prefab_aggregate_fields(
    data_types: &mut Vec<(Type, bool)>,
    fields: &Fields,
) -> (Vec<TokenStream>, Vec<Option<TokenStream>>) {
    let mut subs = Vec::new();
    let mut add_to_entity = Vec::new();
    for field in fields.iter() {
        let is_component = have_component_attribute(&field.attrs[..]);
        let i = match data_types
            .iter()
            .position(|t| t.0 == field.ty && t.1 == is_component)
        {
            Some(i) => i,
            None => {
                data_types.push((field.ty.clone(), is_component));
                data_types.len() - 1
            }
        };
        let tuple_index = Literal::usize_unsuffixed(i);
        let name = field
            .ident
            .as_ref()
            .expect("PrefabData derive only support named fields")
            .clone();
        if is_component {
            subs.push(None);
            add_to_entity.push(quote! {
                system_data.#tuple_index.insert(entity, #name.clone())?;
            });
        } else {
            subs.push(Some(quote! {
                if #name.load_sub_assets(progress, &mut system_data.#tuple_index)? {
                    ret = true;
                }
            }));
            add_to_entity.push(quote! {
                #name.add_to_entity(entity, &mut system_data.#tuple_index, entities, children)?;
            });
        }
    }
    (add_to_entity, subs)
}

fn prepare_prefab_aggregate_struct(
    data: &DataStruct,
) -> (Vec<(Type, bool)>, TokenStream, TokenStream) {
    let mut data_types = Vec::new();
    let (add_to_entity, subs) = prepare_prefab_aggregate_fields(&mut data_types, &data.fields);
    let extract_fields_add = data.fields.iter().map(|f| {
        let name = &f.ident;
        quote! {
            let #name = &self.#name;
        }
    });
    let extract_fields_sub = data.fields.iter().map(|field| {
        let name = &field.ident;
        if !have_component_attribute(&field.attrs[..]) {
            Some(quote! {
                let #name = &mut self.#name;
            })
        } else {
            None
        }
    });
    (
        data_types,
        quote! {
            #(#extract_fields_add)*
            #(#add_to_entity)*
        },
        quote! {
            #(#extract_fields_sub)*
            #(#subs)*
        },
    )
}

fn prepare_prefab_aggregate_enum(
    base: &Ident,
    data: &DataEnum,
) -> (Vec<(Type, bool)>, TokenStream, TokenStream) {
    let mut data_types = Vec::new();
    let mut subs = Vec::new();
    let mut add_to_entity = Vec::new();

    for variant in &data.variants {
        let (variant_add_to_entity, variant_subs) =
            prepare_prefab_aggregate_fields(&mut data_types, &variant.fields);
        let field_names_add: Vec<_> = variant.fields.iter().map(|field| &field.ident).collect();
        let field_names_sub: Vec<_> = variant
            .fields
            .iter()
            .map(|field| {
                // This unwrap is safe because we have already paniced on unnamed fields
                let ident = field.ident.clone().unwrap();
                if !have_component_attribute(&field.attrs[..]) {
                    quote! {
                        #ident
                    }
                } else {
                    quote! {
                        #ident: _
                    }
                }
            })
            .collect();
        let ident = &variant.ident;
        add_to_entity.push(quote! {
            #base::#ident {#(#field_names_add,)*} => {
                #(#variant_add_to_entity)*
            }
        });
        subs.push(quote! {
            #base::#ident {#(#field_names_sub,)*} => {
                #(#variant_subs)*
            }
        });
    }

    (
        data_types,
        quote! {
            match self {
                #(#add_to_entity,)*
            }
        },
        quote! {
            match self {
                #(#subs,)*
            }
        },
    )
}

fn impl_prefab_data_aggregate(ast: &DeriveInput) -> TokenStream {
    let base = &ast.ident;
    let (data_types, add_to_entity, subs) = match &ast.data {
        Data::Struct(ref s) => prepare_prefab_aggregate_struct(s),
        Data::Enum(ref e) => prepare_prefab_aggregate_enum(base, e),
        _ => panic!("PrefabData aggregate derive only support structs and enums"),
    };
    let system_data = data_types.iter().map(|(ty, is_component)| {
        if *is_component {
            quote! {
                 WriteStorage<'pfd, #ty>
            }
        } else {
            quote! {
                <#ty as PrefabData<'pfd>>::SystemData
            }
        }
    });

    let (_, ty_generics, where_clause) = ast.generics.split_for_impl();
    let lf_tokens = gen_def_lt_tokens(&ast.generics);
    let ty_tokens = gen_def_ty_params(&ast.generics);

    quote! {
        impl<'pfd, #lf_tokens #ty_tokens> PrefabData<'pfd> for #base #ty_generics #where_clause {
            type SystemData = (
                #(#system_data,)*
            );
            type Result = ();

            fn add_to_entity(&self,
                             entity: Entity,
                             system_data: &mut Self::SystemData,
                             entities: &[Entity],
                             children: &[Entity]) -> ::std::result::Result<(), Error> {
                #add_to_entity
                Ok(())
            }

            fn load_sub_assets(&mut self,
                               progress: &mut ProgressCounter,
                               system_data: &mut Self::SystemData) -> ::std::result::Result<bool, Error> {
                let mut ret = false;
                #subs
                Ok(ret)
            }
        }
    }
}

fn gen_def_lt_tokens(generics: &Generics) -> TokenStream {
    let lts: Vec<_> = generics
        .lifetimes()
        .map(|x| {
            let ref lt = x.lifetime;
            let ref bounds = x.bounds;

            if bounds.is_empty() {
                quote! { #lt }
            } else {
                quote! { #lt: #( #bounds )+* }
            }
        })
        .collect();

    quote! { #( #lts ),* }
}

fn gen_def_ty_params(generics: &Generics) -> TokenStream {
    let ty_params: Vec<_> = generics
        .type_params()
        .map(|x| {
            let ref ty = x.ident;
            let ref bounds = x.bounds;

            quote! { #ty: #( #bounds )+* }
        })
        .collect();

    quote! { #( #ty_params ),* }
}

fn have_component_attribute(attrs: &[Attribute]) -> bool {
    for meta in attrs
        .iter()
        .filter(|attr| attr.path.segments[0].ident == "prefab")
        .map(|attr| {
            attr.interpret_meta()
                .expect("prefab attribute incorrectly defined")
        })
    {
        match meta {
            Meta::List(l) => {
                for nested_meta in l.nested.iter() {
                    match *nested_meta {
                        NestedMeta::Meta(Meta::Word(ref word)) => {
                            if word == "Component" {
                                return true;
                            }
                        }
                        _ => panic!("prefab attribute does not contain a single word value"),
                    }
                }
            }
            _ => (),
        };
    }
    false
}
