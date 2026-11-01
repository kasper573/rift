#[derive(Default)]
pub struct OutputLog {
    pub lines: Vec<OutputLine>,
}

#[derive(Clone)]
pub struct OutputLine {
    pub tag: &'static str,
    pub text: String,
}

const OUTPUT_KEPT: usize = 20_000;

impl OutputLog {
    pub fn say(&mut self, tag: &'static str, text: impl Into<String>) {
        let text = text.into();
        eprintln!("[{tag}] {text}");
        self.lines.push(OutputLine { tag, text });
        if self.lines.len() > OUTPUT_KEPT {
            let drop = self.lines.len() - OUTPUT_KEPT;
            self.lines.drain(..drop);
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| format!("[{}] {}", line.tag, line.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
