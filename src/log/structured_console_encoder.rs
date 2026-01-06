use log::{
    Record,
    kv::{Error, Key, Value, VisitSource},
};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::encode::{Color, Encode, Style, Write};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct StructuredConsoleEncoderConfig {
    pub pattern: Option<String>,
}

#[derive(Debug)]
pub struct StructuredConsoleEncoder {
    delegate: PatternEncoder,
}

impl StructuredConsoleEncoder {
    pub fn new(pattern: &str) -> Self {
        Self {
            delegate: PatternEncoder::new(pattern),
        }
    }
}

impl Encode for StructuredConsoleEncoder {
    fn encode(&self, w: &mut dyn Write, record: &Record) -> anyhow::Result<()> {
        self.delegate.encode(w, record)?;

        let mut visitor = TextVisitor { writer: w };
        if let Err(e) = record.key_values().visit(&mut visitor) {
            write!(w, " [KV Error: {}]", e)?;
        }

        w.write_all(b"\n")?;
        Ok(())
    }
}

struct TextVisitor<'a> {
    writer: &'a mut dyn Write,
}

impl<'a, 'kvs> VisitSource<'kvs> for TextVisitor<'a> {
    fn visit_pair(&mut self, key: Key<'kvs>, value: Value<'kvs>) -> Result<(), Error> {
        let _ = self.writer.set_style(Style::new().text(Color::Cyan));
        let _ = write!(self.writer, " {}=", key);

        let _ = self.writer.set_style(&Style::default());
        let _ = write!(self.writer, "{}", value);

        Ok(())
    }
}

pub struct StructuredConsoleEncoderDeserializer;

impl log4rs::config::Deserialize for StructuredConsoleEncoderDeserializer {
    type Trait = dyn Encode;
    type Config = StructuredConsoleEncoderConfig;

    fn deserialize(
        &self,
        config: StructuredConsoleEncoderConfig,
        _: &log4rs::config::Deserializers,
    ) -> anyhow::Result<Box<dyn Encode>> {
        let pattern = config.pattern.as_deref().unwrap_or("{d} {l} {m}");
        Ok(Box::new(StructuredConsoleEncoder::new(pattern)))
    }
}
