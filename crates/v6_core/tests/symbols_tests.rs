use v6_core::symbols::SymbolTable;

#[test]
fn test_global_label() {
    let mut st = SymbolTable::new();
    st.define_label("start", 0x100, "test.asm", 1).unwrap();
    assert_eq!(st.resolve("start"), Some(0x100));
}

#[test]
fn test_constant() {
    let mut st = SymbolTable::new();
    st.define_constant("MAX", 255, "test.asm", 1).unwrap();
    assert_eq!(st.resolve("MAX"), Some(255));
}

#[test]
fn test_variable() {
    let mut st = SymbolTable::new();
    st.define_variable("counter", 10, "test.asm", 1).unwrap();
    assert_eq!(st.resolve("counter"), Some(10));
    st.update_variable("counter", 9).unwrap();
    assert_eq!(st.resolve("counter"), Some(9));
}

#[test]
fn test_local_label() {
    let mut st = SymbolTable::new();
    st.define_label("start", 0x100, "test.asm", 1).unwrap();
    st.define_local_label("loop", 0x110, "test.asm", 5).unwrap();
    assert_eq!(st.resolve_local("loop"), Some(0x110));
}

#[test]
fn test_local_scope_isolation() {
    let mut st = SymbolTable::new();
    st.define_label("func1", 0x100, "test.asm", 1).unwrap();
    st.define_local_label("loop", 0x110, "test.asm", 5).unwrap();
    st.define_label("func2", 0x200, "test.asm", 10).unwrap();
    assert_eq!(st.resolve_local("loop"), None);
}
