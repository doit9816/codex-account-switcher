use serde_json::Value;
use std::io::{Error, ErrorKind, Read, Result as IoResult};

const READ_BUFFER_SIZE: usize = 8 * 1024;
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct SseEvent {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

pub(crate) trait SseTransformer {
    fn transform(&mut self, event: SseEvent) -> Result<Vec<u8>, String>;
    fn finish(&mut self) -> Result<Vec<u8>, String>;
}

pub(crate) struct TransformingSseReader<R, T> {
    decoder: SseDecoder<R>,
    transformer: T,
    output: Vec<u8>,
    output_offset: usize,
    finished: bool,
}

impl<R, T> TransformingSseReader<R, T> {
    pub(crate) fn new(reader: R, transformer: T) -> Self {
        Self {
            decoder: SseDecoder::new(reader),
            transformer,
            output: Vec::new(),
            output_offset: 0,
            finished: false,
        }
    }

    fn replace_output(&mut self, output: Vec<u8>) {
        self.output = output;
        self.output_offset = 0;
    }
}

impl<R: Read, T: SseTransformer> Read for TransformingSseReader<R, T> {
    fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            if self.output_offset < self.output.len() {
                let remaining = &self.output[self.output_offset..];
                let count = remaining.len().min(buffer.len());
                buffer[..count].copy_from_slice(&remaining[..count]);
                self.output_offset += count;
                if self.output_offset == self.output.len() {
                    self.output.clear();
                    self.output_offset = 0;
                }
                return Ok(count);
            }
            if self.finished {
                return Ok(0);
            }
            match self.decoder.next_event()? {
                Some(event) => {
                    let output = self.transformer.transform(event).map_err(transform_error)?;
                    if !output.is_empty() {
                        self.replace_output(output);
                    }
                }
                None => {
                    self.finished = true;
                    let output = self.transformer.finish().map_err(transform_error)?;
                    if !output.is_empty() {
                        self.replace_output(output);
                    }
                }
            }
        }
    }
}

pub(crate) fn encode_sse_event(event: &str, value: Value) -> Result<Vec<u8>, String> {
    let mut output = format!("event: {event}\n").into_bytes();
    output.extend_from_slice(b"data: ");
    output.extend_from_slice(
        serde_json::to_string(&value)
            .map_err(crate::display_err)?
            .as_bytes(),
    );
    output.extend_from_slice(b"\n\n");
    Ok(output)
}

fn transform_error(error: String) -> Error {
    Error::new(ErrorKind::InvalidData, error)
}

struct SseDecoder<R> {
    reader: R,
    pending: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    eof: bool,
}

impl<R> SseDecoder<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
            event: None,
            data: Vec::new(),
            eof: false,
        }
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if self.event.is_none() && self.data.is_empty() {
            return None;
        }
        Some(SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }

    fn process_line(&mut self, raw_line: &[u8]) -> IoResult<Option<SseEvent>> {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.starts_with(b":") {
            return Ok(None);
        }
        let line =
            std::str::from_utf8(line).map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        let (field, value) = line
            .split_once(':')
            .map(|(field, value)| (field, value.strip_prefix(' ').unwrap_or(value)))
            .unwrap_or((line, ""));
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
        Ok(None)
    }
}

impl<R: Read> SseDecoder<R> {
    fn next_event(&mut self) -> IoResult<Option<SseEvent>> {
        loop {
            if let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line = self.pending.drain(..=index).collect::<Vec<_>>();
                if let Some(event) = self.process_line(&line[..line.len().saturating_sub(1)])? {
                    return Ok(Some(event));
                }
                continue;
            }
            if self.eof {
                if !self.pending.is_empty() {
                    let line = std::mem::take(&mut self.pending);
                    if let Some(event) = self.process_line(&line)? {
                        return Ok(Some(event));
                    }
                }
                return Ok(self.dispatch());
            }
            let mut buffer = [0_u8; READ_BUFFER_SIZE];
            let read = self.reader.read(&mut buffer)?;
            if read == 0 {
                self.eof = true;
                continue;
            }
            self.pending.extend_from_slice(&buffer[..read]);
            if self.pending.len() > MAX_SSE_LINE_BYTES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "SSE event line exceeds the supported limit",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[derive(Default)]
    struct EchoTransformer;

    impl SseTransformer for EchoTransformer {
        fn transform(&mut self, event: SseEvent) -> Result<Vec<u8>, String> {
            encode_sse_event(
                event.event.as_deref().unwrap_or("message"),
                serde_json::json!({"data": event.data}),
            )
        }

        fn finish(&mut self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn decodes_split_sse_lines_and_multiline_data() {
        let input = Cursor::new(b"event: example\r\ndata: one\r\ndata: two\r\n\r\n".to_vec());
        let mut reader = TransformingSseReader::new(input, EchoTransformer);
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();

        assert!(output.contains("event: example"));
        assert!(output.contains(r#"one\ntwo"#));
    }
}
