use log::{
    Record,
    kv::{Error, Key, Value, VisitSource},
};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::encode::{Color, Encode, Style, Write};
use serde::Deserialize;
use std::io;

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

        let mut visitor = TextVisitor {
            writer: w,
            io_err: None,
        };

        if let Err(kv_err) = record.key_values().visit(&mut visitor) {
            if let Some(io_err) = visitor.io_err {
                return Err(io_err.into());
            }
            write!(w, " [KV Error: {}]", kv_err)?;
        }

        w.write_all(b"\n")?;
        Ok(())
    }
}

struct TextVisitor<'a> {
    writer: &'a mut dyn Write,
    io_err: Option<io::Error>,
}

impl<'a, 'kvs> VisitSource<'kvs> for TextVisitor<'a> {
    fn visit_pair(&mut self, key: Key<'kvs>, value: Value<'kvs>) -> Result<(), Error> {
        let result = (|| {
            self.writer.set_style(Style::new().text(Color::Cyan))?;
            write!(self.writer, " {}=", key)?;

            self.writer.set_style(&Style::default())?;
            write!(self.writer, "{}", value)?;
            Ok::<(), io::Error>(())
        })();

        if let Err(e) = result {
            self.io_err = Some(e);
            return Err(Error::msg("io error during visit"));
        }

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
