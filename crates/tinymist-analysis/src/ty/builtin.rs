use core::fmt;
use std::path::Path;
use std::sync::LazyLock;

use ecow::{eco_format, EcoString};
use regex::RegexSet;
use strum::{EnumIter, IntoEnumIterator};
use typst::foundations::{CastInfo, Regex};
use typst::layout::Ratio;
use typst::syntax::FileId;
use typst::{
    foundations::{AutoValue, Content, Func, NoneValue, ParamInfo, Type, Value},
    layout::Length,
};

use crate::syntax::Decl;
use crate::ty::*;

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, EnumIter)]
pub enum PathPreference {
    Source { allow_package: bool },
    Wasm,
    Csv,
    Image,
    Json,
    Yaml,
    Xml,
    Toml,
    Csl,
    Bibliography,
    RawTheme,
    RawSyntax,
    Special,
    None,
}

impl PathPreference {
    pub fn ext_matcher(&self) -> &'static RegexSet {
        type RegSet = LazyLock<RegexSet>;

        fn make_regex(patterns: &[&str]) -> RegexSet {
            let patterns = patterns.iter().map(|pattern| format!("(?i)^{pattern}$"));
            RegexSet::new(patterns).unwrap()
        }

        static SOURCE_REGSET: RegSet = RegSet::new(|| make_regex(&["typ", "typc"]));
        static WASM_REGSET: RegSet = RegSet::new(|| make_regex(&["wasm"]));
        static IMAGE_REGSET: RegSet = RegSet::new(|| {
            make_regex(&[
                "ico", "bmp", "png", "webp", "jpg", "jpeg", "jfif", "tiff", "gif", "svg", "svgz",
            ])
        });
        static JSON_REGSET: RegSet = RegSet::new(|| make_regex(&["json", "jsonc", "json5"]));
        static YAML_REGSET: RegSet = RegSet::new(|| make_regex(&["yaml", "yml"]));
        static XML_REGSET: RegSet = RegSet::new(|| make_regex(&["xml"]));
        static TOML_REGSET: RegSet = RegSet::new(|| make_regex(&["toml"]));
        static CSV_REGSET: RegSet = RegSet::new(|| make_regex(&["csv"]));
        static BIB_REGSET: RegSet = RegSet::new(|| make_regex(&["yaml", "yml", "bib"]));
        static CSL_REGSET: RegSet = RegSet::new(|| make_regex(&["csl"]));
        static RAW_THEME_REGSET: RegSet = RegSet::new(|| make_regex(&["tmTheme", "xml"]));
        static RAW_SYNTAX_REGSET: RegSet =
            RegSet::new(|| make_regex(&["tmLanguage", "sublime-syntax"]));

        static ALL_REGSET: RegSet = RegSet::new(|| RegexSet::new([r".*"]).unwrap());
        static ALL_SPECIAL_REGSET: RegSet = RegSet::new(|| {
            RegexSet::new({
                let patterns = SOURCE_REGSET.patterns();
                let patterns = patterns.iter().chain(WASM_REGSET.patterns());
                let patterns = patterns.chain(IMAGE_REGSET.patterns());
                let patterns = patterns.chain(JSON_REGSET.patterns());
                let patterns = patterns.chain(YAML_REGSET.patterns());
                let patterns = patterns.chain(XML_REGSET.patterns());
                let patterns = patterns.chain(TOML_REGSET.patterns());
                let patterns = patterns.chain(CSV_REGSET.patterns());
                let patterns = patterns.chain(BIB_REGSET.patterns());
                let patterns = patterns.chain(CSL_REGSET.patterns());
                let patterns = patterns.chain(RAW_THEME_REGSET.patterns());
                patterns.chain(RAW_SYNTAX_REGSET.patterns())
            })
            .unwrap()
        });

        match self {
            Self::Source { .. } => &SOURCE_REGSET,
            Self::Wasm => &WASM_REGSET,
            Self::Csv => &CSV_REGSET,
            Self::Image => &IMAGE_REGSET,
            Self::Json => &JSON_REGSET,
            Self::Yaml => &YAML_REGSET,
            Self::Xml => &XML_REGSET,
            Self::Toml => &TOML_REGSET,
            Self::Csl => &CSL_REGSET,
            Self::Bibliography => &BIB_REGSET,
            Self::RawTheme => &RAW_THEME_REGSET,
            Self::RawSyntax => &RAW_SYNTAX_REGSET,
            Self::Special => &ALL_SPECIAL_REGSET,
            Self::None => &ALL_REGSET,
        }
    }

    pub fn is_match(&self, path: &Path) -> bool {
        let ext = path.extension().and_then(|ext| ext.to_str());
        ext.is_some_and(|ext| self.ext_matcher().is_match(ext))
    }

    pub fn from_ext(path: &str) -> Option<Self> {
        PathPreference::iter().find(|preference| preference.is_match(std::path::Path::new(path)))
    }
}

impl Ty {
    pub fn from_cast_info(ty: &CastInfo) -> Ty {
        match &ty {
            CastInfo::Any => Ty::Any,
            CastInfo::Value(val, doc) => Ty::Value(InsTy::new_doc(val.clone(), *doc)),
            CastInfo::Type(ty) => Ty::Builtin(BuiltinTy::Type(*ty)),
            CastInfo::Union(types) => {
                Ty::iter_union(UnionIter(vec![types.as_slice().iter()]).map(Self::from_cast_info))
            }
        }
    }

    pub fn from_param_site(func: &Func, param: &ParamInfo) -> Ty {
        use typst::foundations::func::Repr;
        match func.inner() {
            Repr::Element(..) | Repr::Native(..) | Repr::Plugin(..) => {
                if let Some(ty) = param_mapping(func, param) {
                    return ty;
                }
            }
            Repr::Closure(_) => {}
            Repr::With(w) => return Ty::from_param_site(&w.0, param),
        };

        Self::from_cast_info(&param.input)
    }

    pub(crate) fn from_return_site(func: &Func, ty: &'_ CastInfo) -> Self {
        use typst::foundations::func::Repr;
        match func.inner() {
            Repr::Element(elem) => return Ty::Builtin(BuiltinTy::Content(Some(*elem))),
            Repr::Closure(_) | Repr::Plugin(_) => {}
            Repr::With(w) => return Ty::from_return_site(&w.0, ty),
            Repr::Native(_) => {}
        };

        Self::from_cast_info(ty)
    }
}

struct UnionIter<'a>(Vec<std::slice::Iter<'a, CastInfo>>);

impl<'a> Iterator for UnionIter<'a> {
    type Item = &'a CastInfo;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let iter = self.0.last_mut()?;
            if let Some(ty) = iter.next() {
                match ty {
                    CastInfo::Union(types) => {
                        self.0.push(types.as_slice().iter());
                    }
                    _ => return Some(ty),
                }
            } else {
                self.0.pop();
            }
        }
    }
}

// todo: we can write some proto files for builtin sigs
#[derive(Debug, Clone, Copy)]
pub enum BuiltinSig<'a> {
    /// Map a function over a tuple.
    TupleMap(&'a Ty),
    /// Get element of a tuple.
    TupleAt(&'a Ty),
}

/// A package identifier.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageId {
    pub namespace: StrRef,
    pub name: StrRef,
}

impl fmt::Debug for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}/{}", self.namespace, self.name)
    }
}

impl TryFrom<FileId> for PackageId {
    type Error = ();

    fn try_from(value: FileId) -> Result<Self, Self::Error> {
        let Some(spec) = value.package() else {
            return Err(());
        };
        Ok(PackageId {
            namespace: spec.namespace.as_str().into(),
            name: spec.name.as_str().into(),
        })
    }
}

#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuiltinTy {
    Clause,
    Undef,
    Space,
    None,
    Break,
    Continue,
    Infer,
    FlowNone,
    Auto,

    Args,
    Color,
    TextSize,
    TextFont,
    TextFeature,
    TextLang,
    TextRegion,

    Label,
    CiteLabel,
    RefLabel,
    Dir,
    Length,
    Float,

    Stroke,
    Margin,
    Inset,
    Outset,
    Radius,

    Tag(Box<(StrRef, Option<Interned<PackageId>>)>),

    /// A value having a specific type.
    Type(typst::foundations::Type),
    /// A value of some type.
    TypeType(typst::foundations::Type),
    /// A content having a specific element type.
    Content(Option<typst::foundations::Element>),
    /// A value of some element type.
    Element(typst::foundations::Element),

    Module(Interned<Decl>),
    Path(PathPreference),
}

impl fmt::Debug for BuiltinTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clause => f.write_str("Clause"),
            Self::Undef => f.write_str("Undef"),
            Self::Content(ty) => {
                if let Some(ty) = ty {
                    write!(f, "Content({})", ty.name())
                } else {
                    f.write_str("Content")
                }
            }
            Self::Space => f.write_str("Space"),
            Self::None => f.write_str("None"),
            Self::Break => f.write_str("Break"),
            Self::Continue => f.write_str("Continue"),
            Self::Infer => f.write_str("Infer"),
            Self::FlowNone => f.write_str("FlowNone"),
            Self::Auto => f.write_str("Auto"),

            Self::Args => write!(f, "Args"),
            Self::Color => write!(f, "Color"),
            Self::TextSize => write!(f, "TextSize"),
            Self::TextFont => write!(f, "TextFont"),
            Self::TextFeature => write!(f, "TextFeature"),
            Self::TextLang => write!(f, "TextLang"),
            Self::TextRegion => write!(f, "TextRegion"),
            Self::Dir => write!(f, "Dir"),
            Self::Length => write!(f, "Length"),
            Self::Label => write!(f, "Label"),
            Self::CiteLabel => write!(f, "CiteLabel"),
            Self::RefLabel => write!(f, "RefLabel"),
            Self::Float => write!(f, "Float"),
            Self::Stroke => write!(f, "Stroke"),
            Self::Margin => write!(f, "Margin"),
            Self::Inset => write!(f, "Inset"),
            Self::Outset => write!(f, "Outset"),
            Self::Radius => write!(f, "Radius"),
            Self::TypeType(ty) => write!(f, "TypeType({})", ty.short_name()),
            Self::Type(ty) => write!(f, "Type({})", ty.short_name()),
            Self::Element(elem) => elem.fmt(f),
            Self::Tag(tag) => {
                let (name, id) = tag.as_ref();
                if let Some(id) = id {
                    write!(f, "Tag({name:?}) of {id:?}")
                } else {
                    write!(f, "Tag({name:?})")
                }
            }
            Self::Module(decl) => write!(f, "{decl:?}"),
            Self::Path(preference) => write!(f, "Path({preference:?})"),
        }
    }
}

impl BuiltinTy {
    pub fn from_value(builtin: &Value) -> Ty {
        if let Value::Bool(v) = builtin {
            return Ty::Boolean(Some(*v));
        }

        Self::from_builtin(builtin.ty())
    }

    pub fn from_builtin(builtin: Type) -> Ty {
        if builtin == Type::of::<AutoValue>() {
            return Ty::Builtin(Self::Auto);
        }
        if builtin == Type::of::<NoneValue>() {
            return Ty::Builtin(Self::None);
        }
        if builtin == Type::of::<typst::visualize::Color>() {
            return Color.literally();
        }
        if builtin == Type::of::<bool>() {
            return Ty::Builtin(Self::None);
        }
        if builtin == Type::of::<f64>() {
            return Float.literally();
        }
        if builtin == Type::of::<Length>() {
            return Length.literally();
        }
        if builtin == Type::of::<Content>() {
            return Ty::Builtin(Self::Content(Option::None));
        }

        Self::Type(builtin).literally()
    }

    pub(crate) fn describe(&self) -> EcoString {
        let res = match self {
            Self::Clause => "any",
            Self::Undef => "any",
            Self::Content(ty) => {
                return if let Some(ty) = ty {
                    eco_format!("content({})", ty.name())
                } else {
                    "content".into()
                };
            }
            Self::Space => "content",
            Self::None => "none",
            Self::Break => "break",
            Self::Continue => "continue",
            Self::Infer => "any",
            Self::FlowNone => "none",
            Self::Auto => "auto",

            Self::Args => "arguments",
            Self::Color => "color",
            Self::TextSize => "text.size",
            Self::TextFont => "text.font",
            Self::TextFeature => "text.feature",
            Self::TextLang => "text.lang",
            Self::TextRegion => "text.region",
            Self::Dir => "dir",
            Self::Length => "length",
            Self::Float => "float",
            Self::Label => "label",
            Self::CiteLabel => "cite-label",
            Self::RefLabel => "ref-label",
            Self::Stroke => "stroke",
            Self::Margin => "margin",
            Self::Inset => "inset",
            Self::Outset => "outset",
            Self::Radius => "radius",
            Self::TypeType(..) => "type",
            Self::Type(ty) => ty.short_name(),
            Self::Element(ty) => ty.name(),
            Self::Tag(tag) => {
                let (name, id) = tag.as_ref();
                return if let Some(id) = id {
                    eco_format!("tag {name} of {id:?}")
                } else {
                    eco_format!("tag {name}")
                };
            }
            Self::Module(m) => return eco_format!("module({})", m.name()),
            Self::Path(s) => match s {
                PathPreference::None => "[any]",
                PathPreference::Special => "[any]",
                PathPreference::Source { .. } => "[source]",
                PathPreference::Wasm => "[wasm]",
                PathPreference::Csv => "[csv]",
                PathPreference::Image => "[image]",
                PathPreference::Json => "[json]",
                PathPreference::Yaml => "[yaml]",
                PathPreference::Xml => "[xml]",
                PathPreference::Toml => "[toml]",
                PathPreference::Csl => "[csl]",
                PathPreference::Bibliography => "[bib]",
                PathPreference::RawTheme => "[theme]",
                PathPreference::RawSyntax => "[syntax]",
            },
        };

        res.into()
    }
}

use BuiltinTy::*;

fn literally(s: impl FlowBuiltinLiterally) -> Ty {
    s.literally()
}

trait FlowBuiltinLiterally {
    fn literally(self) -> Ty;
}

impl FlowBuiltinLiterally for &str {
    fn literally(self) -> Ty {
        Ty::Value(InsTy::new(Value::Str(self.into())))
    }
}

impl FlowBuiltinLiterally for BuiltinTy {
    fn literally(self) -> Ty {
        Ty::Builtin(self.clone())
    }
}

impl FlowBuiltinLiterally for Ty {
    fn literally(self) -> Ty {
        self
    }
}

// separate by middle
macro_rules! flow_builtin_union_inner {
    ($literal_kind:expr) => {
        literally($literal_kind)
    };
    ($($x:expr),+ $(,)?) => {
        Vec::from_iter([
            $(flow_builtin_union_inner!($x)),*
        ])
    };
}

macro_rules! flow_union {
    // the first one is string
    ($($b:tt)*) => {
        Ty::iter_union(flow_builtin_union_inner!( $($b)* ).into_iter())
    };

}

macro_rules! flow_record {
    ($($name:expr => $ty:expr),* $(,)?) => {
        RecordTy::new(vec![
            $(
                (
                    $name.into(),
                    $ty,
                ),
            )*
        ])
    };
}

pub(super) fn param_mapping(func: &Func, param: &ParamInfo) -> Option<Ty> {
    // todo: remove path params which is compatible with 0.12.0
    match (func.name()?, param.name) {
        // todo: pdf.embed
        ("embed", "path") => Some(literally(Path(PathPreference::None))),
        ("cbor", "path" | "source") => Some(literally(Path(PathPreference::None))),
        ("plugin", "source") => Some(literally(Path(PathPreference::Wasm))),
        ("csv", "path" | "source") => Some(literally(Path(PathPreference::Csv))),
        ("image", "path" | "source") => Some(literally(Path(PathPreference::Image))),
        ("read", "path" | "source") => Some(literally(Path(PathPreference::None))),
        ("json", "path" | "source") => Some(literally(Path(PathPreference::Json))),
        ("yaml", "path" | "source") => Some(literally(Path(PathPreference::Yaml))),
        ("xml", "path" | "source") => Some(literally(Path(PathPreference::Xml))),
        ("toml", "path" | "source") => Some(literally(Path(PathPreference::Toml))),
        ("raw", "theme") => Some(literally(Path(PathPreference::RawTheme))),
        ("raw", "syntaxes") => Some(literally(Path(PathPreference::RawSyntax))),
        ("bibliography" | "cite", "style") => Some(Ty::iter_union([
            literally(Path(PathPreference::Csl)),
            Ty::from_cast_info(&param.input),
        ])),
        ("cite", "key") => Some(Ty::iter_union([literally(CiteLabel)])),
        ("ref", "target") => Some(Ty::iter_union([literally(RefLabel)])),
        ("footnote", "body") => Some(Ty::iter_union([
            literally(RefLabel),
            Ty::from_cast_info(&param.input),
        ])),
        ("link", "dest") => {
            static LINK_DEST_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                flow_union!(
                    literally(RefLabel),
                    Ty::Builtin(BuiltinTy::Type(Type::of::<foundations::Str>())),
                    Ty::Builtin(BuiltinTy::Type(Type::of::<typst::introspection::Location>())),
                    Ty::Dict(RecordTy::new(vec![
                        ("x".into(), literally(Length)),
                        ("y".into(), literally(Length)),
                    ])),
                )
            });
            Some(LINK_DEST_TYPE.clone())
        }
        ("bibliography", "path" | "sources") => {
            static BIB_PATH_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                let bib_path_ty = literally(Path(PathPreference::Bibliography));
                Ty::iter_union([bib_path_ty.clone(), Ty::Array(bib_path_ty.into())])
            });
            Some(BIB_PATH_TYPE.clone())
        }
        ("text", "size") => Some(literally(TextSize)),
        ("text", "font") => {
            // todo: the dict can be completed, but we have bugs...
            static FONT_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                Ty::iter_union([literally(TextFont), Ty::Array(literally(TextFont).into())])
            });
            Some(FONT_TYPE.clone())
        }
        ("text", "feature") => {
            static FONT_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                Ty::iter_union([
                    // todo: the key can only be the text feature
                    Ty::Builtin(BuiltinTy::Type(Type::of::<foundations::Dict>())),
                    Ty::Array(literally(TextFeature).into()),
                ])
            });
            Some(FONT_TYPE.clone())
        }
        ("text", "costs") => {
            static FONT_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                Ty::Dict(flow_record!(
                    "hyphenation" => literally(BuiltinTy::Type(Type::of::<Ratio>())),
                    "runt" => literally(BuiltinTy::Type(Type::of::<Ratio>())),
                    "widow" => literally(BuiltinTy::Type(Type::of::<Ratio>())),
                    "orphan" => literally(BuiltinTy::Type(Type::of::<Ratio>())),
                ))
            });
            Some(FONT_TYPE.clone())
        }
        ("text", "lang") => Some(literally(TextLang)),
        ("text", "region") => Some(literally(TextRegion)),
        ("text" | "stack", "dir") => Some(literally(Dir)),
        ("par", "first-line-indent") => {
            static FIRST_LINE_INDENT: LazyLock<Ty> = LazyLock::new(|| {
                Ty::iter_union([
                    literally(Length),
                    Ty::Dict(RecordTy::new(vec![
                        ("amount".into(), literally(Length)),
                        ("all".into(), Ty::Boolean(Option::None)),
                    ])),
                ])
            });
            Some(FIRST_LINE_INDENT.clone())
        }
        (
            // todo: polygon.regular
            "page" | "highlight" | "text" | "path" | "curve" | "rect" | "ellipse" | "circle"
            | "polygon" | "box" | "block" | "table" | "regular",
            "fill",
        ) => Some(literally(Color)),
        (
            // todo: table.cell
            "table" | "cell" | "block" | "box" | "circle" | "ellipse" | "rect" | "square",
            "inset",
        ) => Some(literally(Inset)),
        ("block" | "box" | "circle" | "ellipse" | "rect" | "square", "outset") => {
            Some(literally(Outset))
        }
        ("block" | "box" | "rect" | "square" | "highlight", "radius") => Some(literally(Radius)),
        ("grid" | "table", "columns" | "rows" | "gutter" | "column-gutter" | "row-gutter") => {
            static COLUMN_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                flow_union!(
                    Ty::Value(InsTy::new(Value::Auto)),
                    Ty::Value(InsTy::new(Value::Type(Type::of::<i64>()))),
                    literally(Length),
                    Ty::Array(literally(Length).into()),
                )
            });
            Some(COLUMN_TYPE.clone())
        }
        ("pattern" | "tiling", "size") => {
            static PATTERN_SIZE_TYPE: LazyLock<Ty> = LazyLock::new(|| {
                flow_union!(
                    Ty::Value(InsTy::new(Value::Auto)),
                    Ty::Array(Ty::Builtin(Length).into()),
                )
            });
            Some(PATTERN_SIZE_TYPE.clone())
        }
        ("stroke", "dash") => Some(FLOW_STROKE_DASH_TYPE.clone()),
        (
            //todo: table.cell, table.hline, table.vline, math.cancel, grid.cell, polygon.regular
            "cancel" | "highlight" | "overline" | "strike" | "underline" | "text" | "path"
            | "curve" | "rect" | "ellipse" | "circle" | "polygon" | "box" | "block" | "table"
            | "line" | "cell" | "hline" | "vline" | "regular",
            "stroke",
        ) => Some(Ty::Builtin(Stroke)),
        ("page", "margin") => Some(Ty::Builtin(Margin)),
        _ => Option::None,
    }
}

static FLOW_STROKE_DASH_TYPE: LazyLock<Ty> = LazyLock::new(|| {
    flow_union!(
        "solid",
        "dotted",
        "densely-dotted",
        "loosely-dotted",
        "dashed",
        "densely-dashed",
        "loosely-dashed",
        "dash-dotted",
        "densely-dash-dotted",
        "loosely-dash-dotted",
        Ty::Array(flow_union!("dot", literally(Float)).into()),
        Ty::Dict(flow_record!(
            "array" => Ty::Array(flow_union!("dot", literally(Float)).into()),
            "phase" => literally(Length),
        ))
    )
});

pub static FLOW_STROKE_DICT: LazyLock<Interned<RecordTy>> = LazyLock::new(|| {
    flow_record!(
        "paint" => literally(Color),
        "thickness" => literally(Length),
        "cap" => flow_union!("butt", "round", "square"),
        "join" => flow_union!("miter", "round", "bevel"),
        "dash" => FLOW_STROKE_DASH_TYPE.clone(),
        "miter-limit" => literally(Float),
    )
});

pub static FLOW_MARGIN_DICT: LazyLock<Interned<RecordTy>> = LazyLock::new(|| {
    flow_record!(
        "top" => literally(Length),
        "right" => literally(Length),
        "bottom" => literally(Length),
        "left" => literally(Length),
        "inside" => literally(Length),
        "outside" => literally(Length),
        "x" => literally(Length),
        "y" => literally(Length),
        "rest" => literally(Length),
    )
});

pub static FLOW_INSET_DICT: LazyLock<Interned<RecordTy>> = LazyLock::new(|| {
    flow_record!(
        "top" => literally(Length),
        "right" => literally(Length),
        "bottom" => literally(Length),
        "left" => literally(Length),
        "x" => literally(Length),
        "y" => literally(Length),
        "rest" => literally(Length),
    )
});

pub static FLOW_OUTSET_DICT: LazyLock<Interned<RecordTy>> = LazyLock::new(|| {
    flow_record!(
        "top" => literally(Length),
        "right" => literally(Length),
        "bottom" => literally(Length),
        "left" => literally(Length),
        "x" => literally(Length),
        "y" => literally(Length),
        "rest" => literally(Length),
    )
});

pub static FLOW_RADIUS_DICT: LazyLock<Interned<RecordTy>> = LazyLock::new(|| {
    flow_record!(
        "top" => literally(Length),
        "right" => literally(Length),
        "bottom" => literally(Length),
        "left" => literally(Length),
        "top-left" => literally(Length),
        "top-right" => literally(Length),
        "bottom-left" => literally(Length),
        "bottom-right" => literally(Length),
        "rest" => literally(Length),
    )
});

pub static FLOW_TEXT_FONT_DICT: LazyLock<Interned<RecordTy>> = LazyLock::new(|| {
    flow_record!(
        "name" => literally(TextFont),
        "covers" => flow_union!("latin-in-cjk", BuiltinTy::Type(Type::of::<Regex>())),
    )
});

// todo bad case: array.fold
// todo bad case: datetime
// todo bad case: selector
// todo: function signatures, for example: `locate(loc => ...)`

// todo: numbering/supplement
// todo: grid/table.fill/align/stroke/inset can be a function
// todo: math.cancel.angle can be a function
// todo: math.mat.augment
// todo: csv.row-type can be an array or a dictionary
// todo: text.stylistic-set is an array of integer
// todo: raw.lang can be completed
// todo: smartquote.quotes can be an array or a dictionary
// todo: mat.augment can be a dictionary
// todo: pdf.embed mime-type can be special

// ISO 639

#[cfg(test)]
mod tests {

    use crate::syntax::Decl;

    use super::{SigTy, Ty, TypeVar};

    #[test]
    fn test_image_extension() {
        let path = "test.png";
        let preference = super::PathPreference::from_ext(path).unwrap();
        assert_eq!(preference, super::PathPreference::Image);
    }

    #[test]
    fn test_image_extension_uppercase() {
        let path = "TEST.PNG";
        let preference = super::PathPreference::from_ext(path).unwrap();
        assert_eq!(preference, super::PathPreference::Image);
    }

    // todo: map function
    // Technical Note for implementing a map function:
    // `u`, `v` is in level 2
    // instantiate a `v` as the return type of the map function.
    #[test]
    fn test_map() {
        let u = Ty::Var(TypeVar::new("u".into(), Decl::lit("u").into()));
        let v = Ty::Var(TypeVar::new("v".into(), Decl::lit("v").into()));
        let mapper_fn =
            Ty::Func(SigTy::new([u].into_iter(), None, None, None, Some(v.clone())).into());
        let map_fn =
            Ty::Func(SigTy::new([mapper_fn].into_iter(), None, None, None, Some(v)).into());
        let _ = map_fn;
        // println!("{map_fn:?}");
    }
}
