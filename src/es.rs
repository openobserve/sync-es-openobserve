use crate::Args;
use elasticsearch::auth::Credentials;
use elasticsearch::http::transport::{SingleNodeConnectionPool, TransportBuilder};
use elasticsearch::{ClearScrollParts, Elasticsearch, ScrollParts, SearchParts};
use reqwest::Url;
use serde_json::{json, Value};

static SCROLL_DURATION: &str = "10m";

pub(crate) struct Es {
    pub(crate) client: Elasticsearch,
    index: String,
    query: String,
}

impl Es {
    pub(crate) fn new(args: Args) -> Self {
        let url = Url::parse(&args.es_addr).unwrap();
        let conn_pool = SingleNodeConnectionPool::new(url);

        // 构建 TransportBuilder
        let builder = TransportBuilder::new(conn_pool)
            .auth(Credentials::Basic(args.es_user, args.es_pass))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();

        let client = Elasticsearch::new(builder);

        Es {
            client,
            index: args.es_index,
            query: args.query,
        }
    }

    pub(crate) async fn search(&self, batch_size: usize) -> Result<(String, Vec<Value>, u64), Box<dyn std::error::Error>> {
        let parsed_query: Value = serde_json::from_str(self.query.as_str())?;
        let response = self.client
            .search(SearchParts::Index(&[&self.index]))
            .scroll(SCROLL_DURATION)
            .timeout("10s")
            .size(batch_size as i64)
            .body(parsed_query)
            .send()
            .await?;

        let response_body = response.json::<Value>().await?;
        
        Self::extract_search_result(response_body)
    }
    
    pub(crate) async fn scroll_with_retry(&self, scroll_id: String, max_retries: i32) -> Result<(String, Vec<Value>, u64), Box<dyn std::error::Error>> {
        let mut retries = max_retries;
        loop {
            match self.scroll(scroll_id.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if retries == 0 {
                        return Err(e);
                    }
                    println!("Retrying scroll due to error: {}", e);
                    retries -= 1;
                }
            }
        }
    }

    pub(crate) async fn scroll(&self, scroll_id: String) -> Result<(String, Vec<Value>, u64), Box<dyn std::error::Error>> {
        let response = self.client
            .scroll(ScrollParts::ScrollId(&scroll_id))
            .scroll(SCROLL_DURATION)
            .send()
            .await?;

        let response_body = response.json::<Value>().await?;
        
        Self::extract_search_result(response_body)
    }

    pub(crate) async fn clear_scroll(&self, scroll_id: String) -> Result<(), Box<dyn std::error::Error>> {
        let response = self.client
            .clear_scroll(ClearScrollParts::None)
            .body(json!({
                "scroll_id": scroll_id
                })
            ).send().await?;

        if response.status_code().is_success() {
            Ok(())
        } else {
            Err("Failed to clear scroll".into())
        }
    }
    
    pub(crate) fn extract_search_result(response_body: Value) -> Result<(String, Vec<Value>, u64), Box<dyn std::error::Error>> {
        if response_body["error"].is_object() {
            println!("Error: {:?}", response_body["error"]);
            return Err("Error in response".into());
        }
    
        let scroll_id = response_body["_scroll_id"].as_str().unwrap().to_string();
        let hits = response_body["hits"]["hits"].as_array().unwrap().to_vec();
        let total = response_body["hits"]["total"]["value"].as_u64().unwrap_or(0);
    
        Ok((scroll_id, hits, total))
    }
}