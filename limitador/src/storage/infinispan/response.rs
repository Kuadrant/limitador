pub async fn response_to_string(response: reqwest::Response) -> String {
    response
        .text_with_charset("utf-8")
        .await
        .expect("Error returning response with utf-8 character set")
}
