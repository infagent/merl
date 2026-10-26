use std::process::{Command, Output};

pub struct Installation {
    output: Vec<Output>,
}

impl Installation {
    pub fn given_the_built_binary() -> Self {
        Self { output: Vec::new() }
    }

    pub fn when_its_version_is_requested(mut self) -> Self {
        for flag in ["--version", "-V"] {
            self.output.push(
                Command::new(env!("CARGO_BIN_EXE_merl"))
                    .arg(flag)
                    .output()
                    .unwrap(),
            );
        }
        self
    }

    pub fn then_the_package_version_is_printed_successfully(self) {
        for output in self.output {
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("merl {}\n", env!("CARGO_PKG_VERSION"))
            );
        }
    }
}
