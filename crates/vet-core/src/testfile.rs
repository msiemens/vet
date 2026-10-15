//! Whether a file is a test by the conventions of its language (derived, SPEC I6): its path and
//! whether a container it is in is a `tests` module. Nothing here comes from a language server, so
//! the answer is a conclusion Vet draws by file name, not a primitive.

/// Vet's conclusion that `path` is a test file by the conventions of the languages, or that one of
/// `containers` (the namespaces, types and functions a symbol is in) is a `tests` module.
pub fn is_test(path: &str, containers: &[String]) -> bool {
    if containers.iter().any(|c| c == "tests" || c == "test") {
        return true;
    }
    let (dirs, file) = match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    };
    // A directory named like a test one, or one that ends that way (`HES.Application.Tests`).
    for dir in dirs.split('/') {
        let lower = dir.to_lowercase();
        if lower == "test"
            || lower == "tests"
            || lower == "__tests__"
            || lower.ends_with(".test")
            || lower.ends_with(".tests")
        {
            return true;
        }
    }
    // `Tests.cs`, `Spec.kt` and the like; `test_*.py` at the module level.
    if file.starts_with("test_") && file.ends_with(".py") {
        return true;
    }
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    if matches!(ext, "cs" | "java" | "kt" | "scala")
        && (stem.ends_with("Test") || stem.ends_with("Tests") || stem.ends_with("Spec"))
    {
        return true;
    }
    // `a.test.ts`, `a.spec.tsx` and the extensions TypeScript and JavaScript use.
    if matches!(
        ext,
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "mjsx" | "mts" | "mtsx" | "cjs" | "cjsx" | "cts" | "ctsx"
    ) && (stem.ends_with(".test") || stem.ends_with(".spec"))
    {
        return true;
    }
    // `a_test.rs`, `a_test.go`, `a_test.py`.
    matches!(ext, "rs" | "go" | "py") && stem.ends_with("_test")
}

#[cfg(test)]
mod tests {
    use super::is_test;

    fn named(path: &str, containers: &[&str]) -> bool {
        is_test(path, &containers.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn a_test_is_found_by_the_conventions_of_its_language() {
        // The cases the UI's rule knew, value for value.
        assert!(named("tests/ledger_flow.rs", &[]));
        assert!(named("src/Relays/FooTests.cs", &[]));
        assert!(named("tests/HES.Application.Tests/Foo.cs", &[]));
        assert!(named("ui/src/a.test.ts", &[]));
        assert!(named("pkg/a_test.go", &[]));
        assert!(named("src/retry.rs", &["tests", "next_delay"]));
        assert!(!named("src/Contest.cs", &[]));
        assert!(!named("src/retry.rs", &["RetryPolicy"]));
    }

    #[test]
    fn each_path_convention_stands_on_its_own() {
        // A `test`/`tests`/`__tests__` directory, whatever the case.
        for path in [
            "test/a.rs",
            "tests/a.rs",
            "__tests__/a.rs",
            "src/TESTS/a.rs",
            "src/Test/a.rs",
        ] {
            assert!(named(path, &[]), "{path}");
        }
        // A directory that ends in `.test`/`.tests`.
        assert!(named("HES.Application.Tests/Foo.cs", &[]));
        assert!(named("app.test/x.ts", &[]));
        // A file named after the language's test convention.
        for path in [
            "src/FooTest.cs",
            "src/FooTests.java",
            "src/FooSpec.kt",
            "src/FooSpec.scala",
        ] {
            assert!(named(path, &[]), "{path}");
        }
        for path in [
            "a.test.ts",
            "a.spec.tsx",
            "a.test.mjs",
            "a.spec.cjs",
            "src/b/a.test.cts",
        ] {
            assert!(named(path, &[]), "{path}");
        }
        for path in [
            "a_test.rs",
            "a_test.go",
            "a_test.py",
            "src/a_test.py",
            "test_foo.py",
            "src/test_foo.py",
        ] {
            assert!(named(path, &[]), "{path}");
        }
        // A `tests` module the file is in, whatever the file is called.
        assert!(named("src/anything.rs", &["A", "tests"]));
        assert!(named("src/anything.rs", &["test"]));
    }

    #[test]
    fn production_code_is_not_a_test() {
        for path in [
            "src/retry.rs",
            "src/Contest.cs",
            "src/latest.py",
            "src/tests.rs", // a file named `tests`, not a directory
            "docs/testing.md",
            "src/Specification.cs",
        ] {
            assert!(!named(path, &[]), "{path}");
        }
        assert!(!named("src/retry.rs", &["RetryPolicy"]));
        assert!(!named("src/retry.rs", &["Tests"])); // the container name is exact
    }
}
