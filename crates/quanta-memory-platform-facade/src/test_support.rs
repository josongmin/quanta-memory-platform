use std::process::Command;

/// Validate the actual child filter before a successful exit can certify it.
pub(crate) trait SingleTestCommand {
    fn require_one_test(&mut self) -> &mut Self;
}

impl SingleTestCommand for Command {
    fn require_one_test(&mut self) -> &mut Self {
        let stdout = selected_test_listing(self);
        let count = stdout.lines().filter(|line| line.ends_with(": test")).count();
        assert_eq!(count, 1, "isolated child must select exactly one test: {stdout}");
        // An explicitly selected child must execute even if its test later
        // becomes ignored; a skipped child cannot prove destructor safety.
        self.arg("--include-ignored");
        self
    }
}

fn selected_test_listing(command: &Command) -> String {
    let mut listing = Command::new(command.get_program());
    listing.args(command.get_args()).arg("--list");
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => {
                listing.env(key, value);
            }
            None => {
                listing.env_remove(key);
            }
        }
    }
    if let Some(directory) = command.get_current_dir() {
        listing.current_dir(directory);
    }
    let output = listing.output().expect("list isolated child tests");
    assert!(output.status.success(), "child test listing failed: {output:?}");
    String::from_utf8(output.stdout).expect("UTF-8 test listing")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_child_selection_rejects_zero_tests() {
        let rejected = std::panic::catch_unwind(|| {
            Command::new(std::env::current_exe().expect("test binary"))
                .arg("--exact")
                .arg("test_support::tests::nonexistent_child")
                .require_one_test();
        });
        assert!(rejected.is_err(), "zero-test success must not certify the child");
    }

    #[test]
    fn isolated_child_selection_rejects_multiple_tests() {
        let mut command = Command::new(std::env::current_exe().expect("test binary"));
        command.arg("test_support::tests::isolated_child_selection_");
        assert_eq!(
            selected_test_listing(&command).lines().filter(|line| line.ends_with(": test")).count(),
            2,
            "the rejection must exercise an actually ambiguous filter"
        );
        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            command.require_one_test();
        }));
        assert!(rejected.is_err(), "an ambiguous filter must not certify the child");
    }
}
