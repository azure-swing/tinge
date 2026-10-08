//! ASC parameter exchange: strict XML structure, native OCIO parameter reader, lossless Rust writer.
use crate::{CdlStyle, NodeGrade};
use anyhow::{Context, Result, ensure};
use ocio_rs::{
    FormatMetadata,
    transform::{CDLTransform, Transform},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::{collections::BTreeSet, io::Read, path::Path};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CORRECTIONS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdlFormat {
    Cc,
    Ccc,
    Cdl,
}
impl CdlFormat {
    pub fn from_path(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|v| v.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("cc") => Ok(Self::Cc),
            Some("ccc") => Ok(Self::Ccc),
            Some("cdl") => Ok(Self::Cdl),
            _ => anyhow::bail!("CDL file must use .cc, .ccc or .cdl"),
        }
    }
    fn writer(self) -> &'static str {
        match self {
            Self::Cc => "ColorCorrection",
            Self::Ccc => "ColorCorrectionCollection",
            Self::Cdl => "ColorDecisionList",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Cc => "cc",
            Self::Ccc => "ccc",
            Self::Cdl => "cdl",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CdlMetadata {
    pub descriptions: Vec<String>,
    pub input_descriptions: Vec<String>,
    pub viewing_descriptions: Vec<String>,
    pub sop_descriptions: Vec<String>,
    pub sat_descriptions: Vec<String>,
}
fn text(value: &str) -> Result<()> {
    ensure!(
        value.len() <= 65536
            && value.chars().all(
                |c| matches!(c as u32,9|10|13|0x20..=0xd7ff|0xe000..=0xfffd|0x10000..=0x10ffff)
            ),
        "invalid or oversized XML 1.0 text"
    );
    Ok(())
}
impl CdlMetadata {
    fn groups(&self) -> [(&'static str, &Vec<String>); 5] {
        [
            ("Description", &self.descriptions),
            ("InputDescription", &self.input_descriptions),
            ("ViewingDescription", &self.viewing_descriptions),
            ("SOPDescription", &self.sop_descriptions),
            ("SATDescription", &self.sat_descriptions),
        ]
    }
    fn validate(&self, collection: bool) -> Result<()> {
        ensure!(
            !collection || (self.sop_descriptions.is_empty() && self.sat_descriptions.is_empty()),
            "collection metadata cannot contain SOP/SAT descriptions"
        );
        for (_, values) in self.groups() {
            ensure!(values.len() <= 256, "too many CDL descriptions");
            for value in values {
                text(value)?;
            }
        }
        Ok(())
    }
    fn read(metadata: Option<FormatMetadata>) -> Result<Self> {
        let mut result = Self::default();
        if let Some(metadata) = metadata {
            for i in 0..metadata.num_children() {
                let child = metadata
                    .try_child_element(i)?
                    .context("missing CDL metadata child")?;
                let name = child
                    .element_name()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let values = match name.as_str() {
                    "description" => &mut result.descriptions,
                    "inputdescription" => &mut result.input_descriptions,
                    "viewingdescription" => &mut result.viewing_descriptions,
                    "sopdescription" => &mut result.sop_descriptions,
                    "satdescription" => &mut result.sat_descriptions,
                    _ => anyhow::bail!("unsupported native CDL metadata: {name}"),
                };
                values.push(child.element_value().unwrap_or_default());
            }
        }
        result.validate(false)?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdlCorrection {
    #[serde(default)]
    pub id: String,
    pub slope: [f64; 3],
    pub offset: [f64; 3],
    pub power: [f64; 3],
    pub saturation: f64,
    #[serde(default)]
    pub metadata: CdlMetadata,
}
impl CdlCorrection {
    fn validate(&self) -> Result<()> {
        text(&self.id)?;
        self.metadata.validate(false)?;
        self.grade("exchange validation".into(), CdlStyle::Asc, false, None)
            .validate()
    }
    fn grade(
        &self,
        color_space: String,
        style: CdlStyle,
        inverse: bool,
        exchange: Option<Box<CdlProvenance>>,
    ) -> NodeGrade {
        NodeGrade::Cdl {
            color_space,
            slope: self.slope,
            offset: self.offset,
            power: self.power,
            saturation: self.saturation,
            style,
            inverse,
            exchange,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdlDocument {
    pub format: CdlFormat,
    #[serde(default)]
    pub metadata: CdlMetadata,
    pub corrections: Vec<CdlCorrection>,
}
impl CdlDocument {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.corrections.is_empty() && self.corrections.len() <= MAX_CORRECTIONS,
            "CDL document must contain 1..4096 corrections"
        );
        ensure!(
            self.format != CdlFormat::Cc
                || (self.corrections.len() == 1 && self.metadata == CdlMetadata::default()),
            ".cc requires one correction and no collection metadata"
        );
        self.metadata.validate(true)?;
        let mut ids = BTreeSet::new();
        for correction in &self.corrections {
            correction.validate()?;
            ensure!(
                correction.id.is_empty() || ids.insert(&correction.id),
                "duplicate CDL ID: {}",
                correction.id
            );
        }
        Ok(())
    }
    pub fn import(
        &self,
        source_hash: &str,
        selector: Option<&CdlSelector>,
        color_space: String,
        style: CdlStyle,
        inverse: bool,
    ) -> Result<(usize, NodeGrade)> {
        self.validate()?;
        let index = match selector {
            Some(CdlSelector::Index { index }) => *index as usize,
            Some(CdlSelector::Id { id }) => {
                ensure!(!id.is_empty(), "CDL ID selector must be nonempty");
                self.corrections
                    .iter()
                    .position(|c| &c.id == id)
                    .context("CDL ID not found (case-sensitive)")?
            }
            None => {
                ensure!(
                    self.corrections.len() == 1,
                    "multiple CDLs require an explicit ID or index selector"
                );
                0
            }
        };
        let correction = self
            .corrections
            .get(index)
            .context("CDL index out of range")?;
        let exchange = CdlProvenance {
            source_hash: source_hash.into(),
            source_format: self.format,
            correction_id: correction.id.clone(),
            correction_index: index as u32,
            collection_metadata: self.metadata.clone(),
            correction_metadata: correction.metadata.clone(),
        };
        let grade = correction.grade(color_space, style, inverse, Some(Box::new(exchange)));
        grade.validate()?;
        Ok((index, grade))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CdlSelector {
    Id { id: String },
    Index { index: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdlProvenance {
    pub source_hash: String,
    pub source_format: CdlFormat,
    pub correction_id: String,
    pub correction_index: u32,
    pub collection_metadata: CdlMetadata,
    pub correction_metadata: CdlMetadata,
}
impl CdlProvenance {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source_hash.len() == 64 && self.source_hash.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid CDL provenance hash"
        );
        ensure!(
            self.correction_index < MAX_CORRECTIONS as u32,
            "invalid CDL provenance index"
        );
        text(&self.correction_id)?;
        self.collection_metadata.validate(true)?;
        self.correction_metadata.validate(false)
    }
}

fn validate_xml_shape(xml: &str, format: CdlFormat) -> Result<()> {
    let document = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            nodes_limit: 262144,
            ..Default::default()
        },
    )?;
    let start = xml.trim_start_matches('\u{feff}');
    if let Some(declaration) = start
        .strip_prefix("<?xml")
        .and_then(|s| s.split("?>").next())
        && let Some(encoding) = declaration
            .split_once("encoding")
            .map(|(_, s)| s.trim_start_matches(|c: char| c == '=' || c.is_whitespace()))
    {
        let quote = encoding.chars().next().context("invalid XML encoding")?;
        let name = encoding[quote.len_utf8()..]
            .split(quote)
            .next()
            .unwrap_or_default();
        ensure!(
            name.eq_ignore_ascii_case("utf-8") || name.eq_ignore_ascii_case("utf8"),
            "CDL exchange requires a UTF-8 XML declaration"
        );
    }
    let root = document.root_element();
    ensure!(
        root.tag_name().name() == format.writer(),
        "CDL root element does not match file extension"
    );
    ensure!(
        matches!(
            root.tag_name().namespace(),
            None | Some("urn:ASC:CDL:v1.01" | "urn:ASC:CDL:v1.2")
        ),
        "unsupported ASC CDL namespace"
    );
    let mut count = 0;
    for node in document.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name().name();
        ensure!(
            node.tag_name().namespace() == root.tag_name().namespace(),
            "mixed CDL namespaces are unsupported"
        );
        ensure!(
            node.attributes()
                .all(|a| tag == "ColorCorrection" && a.name() == "id" && a.namespace().is_none()),
            "unsupported attributes on CDL element {tag}"
        );
        let allowed: &[&str] = match tag {
            "ColorCorrectionCollection" => &[
                "Description",
                "InputDescription",
                "ViewingDescription",
                "ColorCorrection",
            ],
            "ColorDecisionList" => &[
                "Description",
                "InputDescription",
                "ViewingDescription",
                "ColorDecision",
            ],
            "ColorDecision" => &["ColorCorrection"],
            "ColorCorrection" => {
                count += 1;
                &[
                    "Description",
                    "InputDescription",
                    "ViewingDescription",
                    "SOPNode",
                    "SatNode",
                ]
            }
            "SOPNode" => &["Description", "Slope", "Offset", "Power"],
            "SatNode" => &["Description", "Saturation"],
            "Description" | "InputDescription" | "ViewingDescription" | "Slope" | "Offset"
            | "Power" | "Saturation" => &[],
            _ => anyhow::bail!("unsupported CDL element: {tag}"),
        };
        let children: Vec<_> = node.children().filter(|n| n.is_element()).collect();
        ensure!(
            children
                .iter()
                .all(|n| allowed.contains(&n.tag_name().name())),
            "unsupported child in CDL element {tag}"
        );
        if !allowed.is_empty() {
            ensure!(
                node.children().filter(|n| n.is_text()).all(|n| n
                    .text()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()),
                "unexpected text inside CDL element {tag}"
            );
        }
        for singleton in [
            "SOPNode",
            "SatNode",
            "Slope",
            "Offset",
            "Power",
            "Saturation",
        ] {
            ensure!(
                children
                    .iter()
                    .filter(|n| n.tag_name().name() == singleton)
                    .count()
                    <= 1,
                "duplicate CDL element {singleton}"
            );
        }
        let required: &[&str] = match tag {
            "SOPNode" => &["Slope", "Offset", "Power"],
            "SatNode" => &["Saturation"],
            "ColorDecision" => &["ColorCorrection"],
            _ => &[],
        };
        for name in required {
            ensure!(
                children
                    .iter()
                    .filter(|n| n.tag_name().name() == *name)
                    .count()
                    == 1,
                "{tag} requires exactly one {name}"
            );
        }
    }
    ensure!(
        count > 0 && count <= MAX_CORRECTIONS,
        "CDL file must contain 1..4096 corrections"
    );
    Ok(())
}
fn parse(bytes: &[u8], format: CdlFormat) -> Result<CdlDocument> {
    ensure!(bytes.len() <= MAX_BYTES, "CDL input exceeds 16 MiB");
    let utf8 = std::str::from_utf8(bytes).context("CDL exchange requires UTF-8 XML")?;
    ensure!(
        !utf8.contains("<!DOCTYPE") && !utf8.contains("<!ENTITY"),
        "CDL DTD/entity declarations are unsupported"
    );
    validate_xml_shape(utf8, format)?;
    // Unique immutable paths avoid OCIO's filename-based CDL cache returning old data.
    let temp = tempfile::tempdir()?;
    let path = temp.path().join(format!("snapshot.{}", format.extension()));
    std::fs::write(&path, bytes)?;
    let group = CDLTransform::create_group_from_file(
        path.to_str().context("CDL snapshot path must be UTF-8")?,
    );
    // Returned transforms own their data; remove temporary filename cache entries
    // on success and failure rather than accumulating them in agent processes.
    ocio_rs::try_clear_all_caches()?;
    let group = group?;
    ensure!(
        group.num_transforms() > 0 && group.num_transforms() as usize <= MAX_CORRECTIONS,
        "CDL file must contain 1..4096 corrections"
    );
    let mut corrections = Vec::new();
    for i in 0..group.num_transforms() {
        let Transform::CDL(cdl) = group.try_transform(i)?.context("missing native CDL")? else {
            anyhow::bail!("native CDL reader returned a non-CDL transform");
        };
        corrections.push(CdlCorrection {
            id: cdl.id().unwrap_or_default(),
            slope: cdl.try_slope()?,
            offset: cdl.try_offset()?,
            power: cdl.try_power_()?,
            saturation: cdl.try_sat()?,
            metadata: CdlMetadata::read(cdl.format_metadata())?,
        });
    }
    let document = CdlDocument {
        format,
        metadata: CdlMetadata::read(group.format_metadata())?,
        corrections,
    };
    document.validate()?;
    Ok(document)
}
pub fn read(path: &Path) -> Result<(CdlDocument, String)> {
    let format = CdlFormat::from_path(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    Ok((parse(&bytes, format)?, hash))
}
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\t', "&#9;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
}
fn descriptions(xml: &mut String, tag: &str, values: &[String]) -> Result<()> {
    for value in values {
        writeln!(xml, "<{tag}>{}</{tag}>", escaped(value))?;
    }
    Ok(())
}
fn common_metadata(xml: &mut String, metadata: &CdlMetadata) -> Result<()> {
    descriptions(xml, "Description", &metadata.descriptions)?;
    descriptions(xml, "InputDescription", &metadata.input_descriptions)?;
    descriptions(xml, "ViewingDescription", &metadata.viewing_descriptions)
}
pub fn write(document: &CdlDocument) -> Result<String> {
    crate::version()?;
    document.validate()?;
    // OCIO 2.5.2 writes CDL numbers with 16 significant digits and double-escapes
    // descriptions containing XML entities. Serialize once with f64 roundtrip
    // precision, then use its reader to verify the actual bytes before export.
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if document.format != CdlFormat::Cc {
        writeln!(
            xml,
            "<{} xmlns=\"urn:ASC:CDL:v1.01\">",
            document.format.writer()
        )?;
        common_metadata(&mut xml, &document.metadata)?;
    }
    for correction in &document.corrections {
        if document.format == CdlFormat::Cdl {
            writeln!(xml, "<ColorDecision>")?;
        }
        writeln!(xml, "<ColorCorrection id=\"{}\">", escaped(&correction.id))?;
        common_metadata(&mut xml, &correction.metadata)?;
        writeln!(xml, "<SOPNode>")?;
        descriptions(
            &mut xml,
            "Description",
            &correction.metadata.sop_descriptions,
        )?;
        for (tag, rgb) in [
            ("Slope", correction.slope),
            ("Offset", correction.offset),
            ("Power", correction.power),
        ] {
            writeln!(
                xml,
                "<{tag}>{:.17e} {:.17e} {:.17e}</{tag}>",
                rgb[0], rgb[1], rgb[2]
            )?;
        }
        writeln!(xml, "</SOPNode>\n<SatNode>")?;
        descriptions(
            &mut xml,
            "Description",
            &correction.metadata.sat_descriptions,
        )?;
        writeln!(
            xml,
            "<Saturation>{:.17e}</Saturation>\n</SatNode>\n</ColorCorrection>",
            correction.saturation
        )?;
        if document.format == CdlFormat::Cdl {
            writeln!(xml, "</ColorDecision>")?;
        }
    }
    if document.format != CdlFormat::Cc {
        writeln!(xml, "</{}>", document.format.writer())?;
    }
    let reparsed = parse(xml.as_bytes(), document.format)?;
    ensure!(
        &reparsed == document,
        "CDL export did not preserve parameters/metadata; output not written"
    );
    Ok(xml)
}

/// Exports stored forward parameters, not the evaluated node graph.
pub fn correction_from_grade(
    grade: &NodeGrade,
    fallback_id: &str,
) -> Result<(CdlCorrection, Option<CdlMetadata>)> {
    grade.validate()?;
    let NodeGrade::Cdl {
        slope,
        offset,
        power,
        saturation,
        exchange,
        ..
    } = grade
    else {
        anyhow::bail!("selected node is not an OCIO CDL");
    };
    Ok((
        CdlCorrection {
            id: exchange
                .as_ref()
                .map(|p| p.correction_id.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| fallback_id.into()),
            slope: *slope,
            offset: *offset,
            power: *power,
            saturation: *saturation,
            metadata: exchange
                .as_ref()
                .map(|p| p.correction_metadata.clone())
                .unwrap_or_default(),
        },
        exchange.as_ref().map(|p| p.collection_metadata.clone()),
    ))
}
