use minotari::ApiDoc;
use utoipa::OpenApi;

#[test]
fn test_openapi_spec_is_up_to_date() {
    let generated_spec_json = ApiDoc::openapi().to_pretty_json().unwrap();

    let committed_spec_path = "openapi.json";
    let committed_spec_json = std::fs::read_to_string(committed_spec_path)
        .expect("Could not read the committed openapi.json file. Did you run `cargo run --bin generate-openapi`?");

    if generated_spec_json != committed_spec_json {
        panic!(
            "The committed openapi.json spec is out of date! \
             Please run `cargo run --bin generate-openapi` and commit the changes."
        );
    }
}
