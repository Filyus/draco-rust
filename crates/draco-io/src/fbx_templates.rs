//! Class defaults an FBX document states once in `Definitions`.
//!
//! An exporter that gives every `Model` the same `RotationOrder`, or every
//! material the same `ShadingModel`, writes that value once as a
//! `PropertyTemplate` and leaves it off the objects themselves. The value is
//! in the file; an object that states nothing is stating "the class default",
//! not "unknown".
//!
//! Reading only the object's own `Properties70` therefore loses data. In this
//! crate's corpus it loses 10128 (object, property) pairs across 181 of the
//! 185 documents that carry templates -- including the shading model of 174 of
//! 188 materials, and every field-of-view and focal length in the Revit files,
//! whose cameras state almost nothing directly.
//!
//! The object always wins. 5553 pairs are declared in both places, `Lcl
//! Translation` on 928 models among them, so a resolver that let the template
//! override would move every one of those objects to the origin without
//! raising anything.

use std::collections::HashMap;

use crate::fbx_node::{FbxNode, FbxProperty};

/// The `PropertyTemplate` blocks of one document, indexed by object type.
pub(crate) struct PropertyTemplates<'a> {
    /// Keyed by the `ObjectType` name, which is literally the object record's
    /// node name (`Model`, `Material`, `NodeAttribute`, ...). The value is the
    /// template's FBX class and its `Properties70` block.
    by_object_type: HashMap<&'a str, (&'a str, &'a FbxNode)>,
}

impl<'a> PropertyTemplates<'a> {
    /// Collects the templates from a document's top-level nodes.
    ///
    /// A document with no `Definitions` yields an empty index, and every
    /// lookup against it returns `None`, which is exactly the behaviour this
    /// crate had before templates were read at all.
    pub(crate) fn build(nodes: &'a [FbxNode]) -> Self {
        let mut by_object_type = HashMap::new();
        for definitions in nodes.iter().filter(|node| node.name == "Definitions") {
            for object_type in definitions
                .children
                .iter()
                .filter(|child| child.name == "ObjectType")
            {
                let Some(FbxProperty::String(type_name)) = object_type.properties.first() else {
                    continue;
                };
                let Some(template) = object_type
                    .children
                    .iter()
                    .find(|child| child.name == "PropertyTemplate")
                else {
                    continue;
                };
                let Some(FbxProperty::String(class)) = template.properties.first() else {
                    continue;
                };
                let Some(properties) = template
                    .children
                    .iter()
                    .find(|child| child.name == "Properties70")
                else {
                    continue;
                };
                // No corpus document declares two templates for one object
                // type, but the format does not forbid it. Keep the first
                // rather than guessing which of two applies.
                by_object_type
                    .entry(type_name.as_str())
                    .or_insert((class.as_str(), properties));
            }
        }
        Self { by_object_type }
    }

    /// The `Properties70` of the template that applies to this object record.
    pub(crate) fn for_object(&self, object: &FbxNode) -> Option<&'a FbxNode> {
        let (class, properties) = self.by_object_type.get(object.name.as_str())?;
        // `NodeAttribute` is the one record name that covers unrelated
        // classes, and a document declares only one template for it. Matching
        // on the record name alone would hand the `FbxCamera` template to the
        // seven `Light` attributes that sit beside one in this corpus, and the
        // `FbxNull` template to 186 `LimbNode`s.
        if object.name == "NodeAttribute" {
            let object_class = match object.properties.get(2) {
                Some(FbxProperty::String(class)) => class.as_str(),
                _ => return None,
            };
            if !attribute_class_matches(class, object_class) {
                return None;
            }
        }
        Some(properties)
    }
}

/// Whether an attribute template describes objects of this class.
///
/// Not derivable from the strings: `FbxSkeleton` describes a `LimbNode` and
/// `FbxNull` a `Null`, so neither dropping the `Fbx` prefix nor adding it
/// resolves the pair.
fn attribute_class_matches(template_class: &str, object_class: &str) -> bool {
    match template_class {
        "FbxCamera" => object_class == "Camera",
        "FbxLight" => object_class == "Light",
        "FbxSkeleton" => matches!(object_class, "LimbNode" | "Limb" | "Root"),
        "FbxNull" => object_class == "Null",
        "FbxLODGroup" => object_class == "LodGroup",
        _ => false,
    }
}

/// One object's properties, with its class defaults behind them.
#[derive(Clone, Copy)]
pub(crate) struct ObjectProperties<'a> {
    object: &'a FbxNode,
    template: Option<&'a FbxNode>,
}

impl<'a> ObjectProperties<'a> {
    pub(crate) fn new(object: &'a FbxNode, templates: &PropertyTemplates<'a>) -> Self {
        Self {
            object,
            template: templates.for_object(object),
        }
    }

    /// The object record itself.
    pub(crate) fn node(&self) -> &'a FbxNode {
        self.object
    }

    /// The class defaults, for a caller that walks `Properties70` itself.
    pub(crate) fn template(&self) -> Option<&'a FbxNode> {
        self.template
    }

    /// The object's own value, or the class default when it states none.
    ///
    /// Never the other way round -- see the module header.
    pub(crate) fn get(&self, name: &str) -> Option<&'a FbxNode> {
        self.object
            .children
            .iter()
            .filter(|child| child.name == "Properties70")
            .find_map(|properties| find_property(properties, name))
            .or_else(|| self.template.and_then(|block| find_property(block, name)))
    }
}

/// The first `P` record in a `Properties70` block with this name.
pub(crate) fn find_property<'a>(properties70: &'a FbxNode, name: &str) -> Option<&'a FbxNode> {
    properties70.children.iter().find(|entry| {
        entry.name == "P"
            && matches!(entry.properties.first(), Some(FbxProperty::String(key)) if key == name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(name: &str, value: f64) -> FbxNode {
        FbxNode {
            name: "P".to_string(),
            properties: vec![
                FbxProperty::String(name.to_string()),
                FbxProperty::String(String::new()),
                FbxProperty::String(String::new()),
                FbxProperty::String(String::new()),
                FbxProperty::F64(value),
            ],
            children: Vec::new(),
        }
    }

    fn properties70(entries: Vec<FbxNode>) -> FbxNode {
        FbxNode {
            name: "Properties70".to_string(),
            properties: Vec::new(),
            children: entries,
        }
    }

    /// `Definitions` holding one template of `class` for `object_type`.
    fn definitions(object_type: &str, class: &str, entries: Vec<FbxNode>) -> FbxNode {
        FbxNode {
            name: "Definitions".to_string(),
            properties: Vec::new(),
            children: vec![FbxNode {
                name: "ObjectType".to_string(),
                properties: vec![FbxProperty::String(object_type.to_string())],
                children: vec![FbxNode {
                    name: "PropertyTemplate".to_string(),
                    properties: vec![FbxProperty::String(class.to_string())],
                    children: vec![properties70(entries)],
                }],
            }],
        }
    }

    fn object(name: &str, class: &str, entries: Vec<FbxNode>) -> FbxNode {
        FbxNode {
            name: name.to_string(),
            properties: vec![
                FbxProperty::I64(1),
                FbxProperty::String("Thing".to_string()),
                FbxProperty::String(class.to_string()),
            ],
            children: vec![properties70(entries)],
        }
    }

    fn value_of(entry: &FbxNode) -> f64 {
        match entry.properties.get(4) {
            Some(FbxProperty::F64(value)) => *value,
            other => panic!("expected an f64 value, got {other:?}"),
        }
    }

    /// The object wins. Getting this backwards moves 928 corpus models to the
    /// origin, silently.
    #[test]
    fn an_object_overrides_the_class_default() {
        let nodes = vec![definitions(
            "Model",
            "FbxNode",
            vec![property("Lcl Translation", 0.0)],
        )];
        let templates = PropertyTemplates::build(&nodes);
        let model = object("Model", "Mesh", vec![property("Lcl Translation", 7.5)]);

        let properties = ObjectProperties::new(&model, &templates);
        assert_eq!(
            value_of(properties.get("Lcl Translation").expect("present")),
            7.5
        );
    }

    #[test]
    fn a_class_default_fills_in_what_the_object_leaves_out() {
        let nodes = vec![definitions(
            "Model",
            "FbxNode",
            vec![property("RotationOrder", 2.0)],
        )];
        let templates = PropertyTemplates::build(&nodes);
        let model = object("Model", "Mesh", Vec::new());

        let properties = ObjectProperties::new(&model, &templates);
        assert_eq!(
            value_of(properties.get("RotationOrder").expect("from the template")),
            2.0
        );
        assert!(properties.get("Lcl Scaling").is_none());
    }

    /// A document declares one `NodeAttribute` template, and this corpus has
    /// `Light` attributes sitting beside an `FbxCamera` one.
    #[test]
    fn an_attribute_template_applies_only_to_its_own_class() {
        let nodes = vec![definitions(
            "NodeAttribute",
            "FbxCamera",
            vec![property("FocalLength", 34.893)],
        )];
        let templates = PropertyTemplates::build(&nodes);

        let camera = object("NodeAttribute", "Camera", Vec::new());
        assert_eq!(
            value_of(
                ObjectProperties::new(&camera, &templates)
                    .get("FocalLength")
                    .expect("the camera template applies to a camera")
            ),
            34.893
        );

        let light = object("NodeAttribute", "Light", Vec::new());
        assert!(
            ObjectProperties::new(&light, &templates)
                .get("FocalLength")
                .is_none(),
            "a camera's focal length must not reach a light"
        );
    }

    /// The class names do not match by string, so the rule has to be a table.
    #[test]
    fn a_skeleton_template_applies_to_a_limb_node() {
        let nodes = vec![definitions(
            "NodeAttribute",
            "FbxSkeleton",
            vec![property("Size", 33.0)],
        )];
        let templates = PropertyTemplates::build(&nodes);

        for class in ["LimbNode", "Limb", "Root"] {
            let limb = object("NodeAttribute", class, Vec::new());
            assert!(
                ObjectProperties::new(&limb, &templates)
                    .get("Size")
                    .is_some(),
                "FbxSkeleton must apply to {class}"
            );
        }
        let null = object("NodeAttribute", "Null", Vec::new());
        assert!(ObjectProperties::new(&null, &templates)
            .get("Size")
            .is_none());
    }

    /// A `Model` template applies whatever the model's class attribute is:
    /// every Model is an FbxNode, and the class names its attribute.
    #[test]
    fn a_model_template_applies_to_every_model_class() {
        let nodes = vec![definitions(
            "Model",
            "FbxNode",
            vec![property("InheritType", 1.0)],
        )];
        let templates = PropertyTemplates::build(&nodes);

        for class in ["Mesh", "LimbNode", "Camera", "Null", "IKEffector"] {
            let model = object("Model", class, Vec::new());
            assert!(
                ObjectProperties::new(&model, &templates)
                    .get("InheritType")
                    .is_some(),
                "the FbxNode template must apply to Model::{class}"
            );
        }
    }

    #[test]
    fn a_document_without_definitions_reads_as_before() {
        let templates = PropertyTemplates::build(&[]);
        let model = object("Model", "Mesh", vec![property("Lcl Scaling", 2.0)]);
        let properties = ObjectProperties::new(&model, &templates);

        assert_eq!(
            value_of(properties.get("Lcl Scaling").expect("present")),
            2.0
        );
        assert!(properties.get("RotationOrder").is_none());
        assert!(properties.template().is_none());
    }
}
