fn main() {
    // sqlx::migrate! embeds the SQL at compile time; rebuild when migrations change.
    println!("cargo:rerun-if-changed=../../deploy/migrations");
}
