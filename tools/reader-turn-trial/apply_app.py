"""Add the opt-in pointer driver to a disposable WeRead measurement source tree."""

import argparse
from pathlib import Path


HERE = Path(__file__).resolve().parent
CFG = '#[cfg(all(target_os = "linux", feature = "kindle"))]'


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"Missing or repeated Rust insertion point: {old!r}")
    return source.replace(old, new)


def add_arm_probe(main):
    entry = 'fn main() -> std::process::ExitCode {\n'
    anchor = entry + '    startup_mark("main_enter");'
    patched = (entry + f'    {CFG}\n    if let Some(status) = trial_reader_turn::probe_build_arm() {{\n'
               '        return status;\n    }\n    startup_mark("main_enter");')
    if patched in main:
        if main.count(patched) != 1 or main.count("trial_reader_turn::probe_build_arm()") != 1:
            raise ValueError("Unexpected repeat of the Reader trial arm probe")
        return main
    if "trial_reader_turn::probe_build_arm()" in main:
        raise ValueError("Refusing an unrecognized arm probe")
    return replace_once(main, anchor, patched)


def apply(source_root):
    root = source_root.resolve(strict=True)
    if (root / ".git").exists() or root == HERE.parents[1]:
        raise ValueError("Use an exported, disposable source tree, not a repository checkout")
    main_path = root / "src/main.rs"
    app_path = root / "src/app/mod.rs"
    driver = root / "src/trial_reader_turn.rs"
    probe = root / "src/app/trial_reader_probe.rs"
    for destination in (driver, probe):
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(destination)
    main = add_arm_probe(main_path.read_text(encoding="utf-8"))
    app = app_path.read_text(encoding="utf-8")
    main = replace_once(main, '#[path = "main_font_budget.rs"]',
                        f'{CFG}\nmod trial_reader_turn;\n\n#[path = "main_font_budget.rs"]')
    main = replace_once(main, "fn run(arguments: Arguments) -> Result<()> {\n",
                        f"fn run(arguments: Arguments) -> Result<()> {{\n"
                        f"    {CFG}\n    let trial_reader = Rc::new(RefCell::new(trial_reader_turn::Driver::configure(&arguments)?));\n")
    main = replace_once(main, "    let timer_application = application.clone();",
                        f"    {CFG}\n    let timed_reader = trial_reader.clone();\n    let timer_application = application.clone();")
    before_quit = '            if STOP_REQUESTED.load(Ordering::Relaxed)\n                || deadline.is_some_and(|limit| elapsed >= limit)'
    main = replace_once(main, before_quit,
                        f"            {CFG}\n"
                        "            if let Some(driver) = timed_reader.borrow_mut().as_mut() {\n"
                        "                match driver.tick(&ui, &timer_application) {\n"
                        "                    Ok(false) => {}\n"
                        "                    Ok(true) => { request_quit(&timer_quitting, &timer_failures); return; }\n"
                        "                    Err(error) => {\n"
                        '                        timer_failures.borrow_mut().push(format!("Reader turn trial: {error:#}"));\n'
                        "                        request_quit(&timer_quitting, &timer_failures);\n"
                        "                        return;\n"
                        "                    }\n"
                        "                }\n"
                        "            }\n" + before_quit)
    end = '    if !messages.is_empty() {\n        bail!("{}", messages.summary());\n    }\n    Ok(())'
    main = replace_once(main, end, end.replace("    Ok(())", f"    {CFG}\n    if let Some(driver) = trial_reader.borrow().as_ref() {{\n        driver.finish()?;\n    }}\n    Ok(())"))
    app = replace_once(app, "mod article_images;\n", f"{CFG}\nmod trial_reader_probe;\nmod article_images;\n")
    driver.write_bytes((HERE / "app_driver.rs").read_bytes())
    probe.write_bytes((HERE / "app_probe.rs").read_bytes())
    main_path.write_text(main, encoding="utf-8")
    app_path.write_text(app, encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    apply(parser.parse_args().source)
