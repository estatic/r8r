//! Node type descriptions served at `/types/nodes.json` (spec §4.3): the
//! `INodeTypeDescription` shape the n8n editor renders forms from. These
//! cover the natively implemented nodes; each lists the parameters r8r
//! reads.

use super::nodes::Registry;
use serde_json::{json, Value};

struct Desc {
    name: &'static str,
    display: &'static str,
    group: &'static str,
    versions: &'static [f64],
    description: &'static str,
    outputs: usize,
    inputs: usize,
    params: &'static [&'static str],
    credentials: &'static [&'static str],
}

const fn d(
    name: &'static str,
    display: &'static str,
    group: &'static str,
    versions: &'static [f64],
    description: &'static str,
    inputs: usize,
    outputs: usize,
    params: &'static [&'static str],
    credentials: &'static [&'static str],
) -> Desc {
    Desc { name, display, group, versions, description, outputs, inputs, params, credentials }
}

const DESCRIPTIONS: &[Desc] = &[
    d("n8n-nodes-base.manualTrigger", "Manual Trigger", "trigger", &[1.0], "Runs the flow on clicking a button in n8n", 0, 1, &["notice"], &[]),
    d("n8n-nodes-base.scheduleTrigger", "Schedule Trigger", "trigger", &[1.0, 1.1, 1.2], "Triggers the workflow on a given schedule", 0, 1, &["rule"], &[]),
    d("n8n-nodes-base.webhook", "Webhook", "trigger", &[1.0, 1.1, 2.0], "Starts the workflow when a webhook is called", 0, 1, &["httpMethod", "path", "authentication", "responseMode", "responseCode", "responseData", "options"], &["httpBasicAuth", "httpHeaderAuth", "jwtAuth"]),
    d("n8n-nodes-base.formTrigger", "n8n Form Trigger", "trigger", &[1.0, 2.0, 2.1, 2.2], "Generate webforms in n8n and pass their responses to the workflow", 0, 1, &["formTitle", "formDescription", "formFields", "options"], &[]),
    d("n8n-nodes-base.errorTrigger", "Error Trigger", "trigger", &[1.0], "Triggers the workflow when another workflow has an error", 0, 1, &["notice"], &[]),
    d("n8n-nodes-base.executeWorkflowTrigger", "Execute Workflow Trigger", "trigger", &[1.0, 1.1], "Helpers for calling other n8n workflows", 0, 1, &["inputSource"], &[]),
    d("n8n-nodes-base.set", "Edit Fields (Set)", "input", &[3.0, 3.1, 3.2, 3.3, 3.4], "Modify, add, or remove item fields", 1, 1, &["mode", "assignments", "jsonOutput", "includeOtherFields", "include", "options"], &[]),
    d("n8n-nodes-base.if", "If", "transform", &[2.0, 2.1, 2.2], "Route items to different branches (true/false)", 1, 2, &["conditions", "looseTypeValidation", "options"], &[]),
    d("n8n-nodes-base.filter", "Filter", "transform", &[2.0, 2.1, 2.2], "Remove items matching a condition", 1, 1, &["conditions", "looseTypeValidation", "options"], &[]),
    d("n8n-nodes-base.switch", "Switch", "transform", &[3.0, 3.1, 3.2], "Route items depending on defined expression or rules", 1, 4, &["mode", "rules", "numberOutputs", "output", "options"], &[]),
    d("n8n-nodes-base.merge", "Merge", "transform", &[3.0, 3.1], "Merges data of multiple streams once data from both is available", 2, 1, &["mode", "combineBy", "fieldsToMatchString", "joinMode", "outputDataFrom", "options"], &[]),
    d("n8n-nodes-base.code", "Code", "transform", &[1.0, 2.0], "Run custom JavaScript or Python code", 1, 1, &["mode", "language", "jsCode", "pythonCode"], &[]),
    d("n8n-nodes-base.httpRequest", "HTTP Request", "output", &[4.0, 4.1, 4.2], "Makes an HTTP request and returns the response data", 1, 1, &["method", "url", "authentication", "sendQuery", "queryParameters", "sendHeaders", "headerParameters", "sendBody", "contentType", "bodyParameters", "jsonBody", "options"], &["httpBasicAuth", "httpHeaderAuth", "httpQueryAuth", "httpBearerAuth", "oAuth2Api"]),
    d("n8n-nodes-base.splitInBatches", "Loop Over Items (Split in Batches)", "organization", &[3.0], "Split data into batches and iterate over each batch", 1, 2, &["batchSize", "options"], &[]),
    d("n8n-nodes-base.splitOut", "Split Out", "transform", &[1.0], "Turn a list inside item(s) into separate items", 1, 1, &["fieldToSplitOut", "include", "fieldsToInclude", "options"], &[]),
    d("n8n-nodes-base.aggregate", "Aggregate", "transform", &[1.0], "Combine a field from many items into a list in a single item", 1, 1, &["aggregate", "fieldsToAggregate", "destinationFieldName", "options"], &[]),
    d("n8n-nodes-base.summarize", "Summarize", "transform", &[1.0, 1.1], "Sum, count, max, etc. across items", 1, 1, &["fieldsToSummarize", "fieldsToSplitBy", "options"], &[]),
    d("n8n-nodes-base.sort", "Sort", "transform", &[1.0], "Change items order", 1, 1, &["type", "sortFieldsUi", "options"], &[]),
    d("n8n-nodes-base.limit", "Limit", "transform", &[1.0], "Restrict the number of items", 1, 1, &["maxItems", "keep"], &[]),
    d("n8n-nodes-base.removeDuplicates", "Remove Duplicates", "transform", &[1.0, 1.1, 2.0], "Delete items with matching field values", 1, 1, &["operation", "compare", "fieldsToCompare", "options"], &[]),
    d("n8n-nodes-base.compareDatasets", "Compare Datasets", "transform", &[1.0, 2.0, 2.1, 2.2, 2.3], "Compare two inputs for changes", 2, 4, &["mergeByFields", "options"], &[]),
    d("n8n-nodes-base.wait", "Wait", "organization", &[1.0, 1.1], "Wait before continue with execution", 1, 1, &["resume", "amount", "unit", "dateTime", "httpMethod", "options"], &[]),
    d("n8n-nodes-base.respondToWebhook", "Respond to Webhook", "transform", &[1.0, 1.1], "Returns data for Webhook", 1, 1, &["respondWith", "responseBody", "redirectURL", "options"], &[]),
    d("n8n-nodes-base.executeWorkflow", "Execute Workflow", "transform", &[1.0, 1.1, 1.2], "Execute another workflow", 1, 1, &["source", "workflowId", "mode", "options"], &[]),
    d("n8n-nodes-base.noOp", "No Operation, do nothing", "organization", &[1.0], "No Operation", 1, 1, &["notice"], &[]),
    d("n8n-nodes-base.stopAndError", "Stop and Error", "input", &[1.0], "Throw an error in the workflow", 1, 0, &["errorType", "errorMessage", "errorObject"], &[]),
    d("n8n-nodes-base.dateTime", "Date & Time", "transform", &[2.0], "Allows you to manipulate date and time values", 1, 1, &["operation", "date", "format", "customFormat", "outputFieldName", "options"], &[]),
    d("n8n-nodes-base.crypto", "Crypto", "transform", &[1.0], "Provide cryptographic utilities", 1, 1, &["action", "type", "value", "secret", "dataPropertyName", "encoding"], &[]),
    d("n8n-nodes-base.markdown", "Markdown", "output", &[1.0], "Convert data between Markdown and HTML", 1, 1, &["mode", "markdown", "html", "destinationKey", "options"], &[]),
    d("n8n-nodes-base.xml", "XML", "transform", &[1.0], "Convert data from and to XML", 1, 1, &["mode", "dataPropertyName", "options"], &[]),
    d("n8n-nodes-base.executeCommand", "Execute Command", "transform", &[1.0], "Executes a command on the host", 1, 1, &["executeOnce", "command"], &[]),
    // GA catalog entries that r8r does not run natively yet: described so
    // imported workflows render in the editor.
    d("n8n-nodes-base.html", "HTML", "transform", &[1.0, 1.2], "Work with HTML", 1, 1, &["operation", "html", "options"], &[]),
    d("n8n-nodes-base.jwt", "JWT", "transform", &[1.0], "JWT", 1, 1, &["operation", "claims", "options"], &["jwtAuth"]),
    d("n8n-nodes-base.compression", "Compression", "transform", &[1.0, 1.1], "Compress and decompress files", 1, 1, &["operation", "binaryPropertyName"], &[]),
    d("n8n-nodes-base.extractFromFile", "Extract from File", "input", &[1.0], "Convert binary data to JSON", 1, 1, &["operation", "binaryPropertyName", "options"], &[]),
    d("n8n-nodes-base.convertToFile", "Convert to File", "input", &[1.0, 1.1], "Convert JSON data to binary data", 1, 1, &["operation", "binaryPropertyName", "options"], &[]),
    d("n8n-nodes-base.readWriteFile", "Read/Write Files from Disk", "input", &[1.0], "Read or write files from the computer that runs n8n", 1, 1, &["operation", "fileSelector", "options"], &[]),
    d("n8n-nodes-base.localFileTrigger", "Local File Trigger", "trigger", &[1.0], "Triggers a workflow on file system changes", 0, 1, &["triggerOn", "path", "events"], &[]),
    d("n8n-nodes-base.emailSend", "Send Email", "output", &[2.0, 2.1], "Sends an email using SMTP protocol", 1, 1, &["fromEmail", "toEmail", "subject", "emailFormat", "options"], &["smtp"]),
    d("n8n-nodes-base.emailReadImap", "Email Trigger (IMAP)", "trigger", &[2.0], "Triggers the workflow when a new email is received", 0, 1, &["mailbox", "postProcessAction", "options"], &["imap"]),
    d("n8n-nodes-base.ftp", "FTP", "input", &[1.0], "Transfer files via FTP or SFTP", 1, 1, &["protocol", "operation", "path"], &["ftp", "sftp"]),
    d("n8n-nodes-base.ssh", "SSH", "input", &[1.0], "Execute commands via SSH", 1, 1, &["resource", "operation", "command"], &["sshPassword", "sshPrivateKey"]),
    d("n8n-nodes-base.postgres", "Postgres", "input", &[2.0, 2.5], "Get, add and update data in Postgres", 1, 1, &["operation", "schema", "table", "query", "options"], &["postgres"]),
    d("n8n-nodes-base.mySql", "MySQL", "input", &[2.0, 2.4], "Get, add and update data in MySQL", 1, 1, &["operation", "table", "query", "options"], &["mySql"]),
    d("n8n-nodes-base.microsoftSql", "Microsoft SQL", "input", &[1.0, 1.1], "Get, add and update data in Microsoft SQL", 1, 1, &["operation", "query"], &["microsoftSql"]),
    d("n8n-nodes-base.mongoDb", "MongoDB", "input", &[1.0, 1.1], "Find, insert and update documents in MongoDB", 1, 1, &["operation", "collection", "query"], &["mongoDb"]),
    d("n8n-nodes-base.redis", "Redis", "input", &[1.0], "Get, send and update data in Redis", 1, 1, &["operation", "key", "value"], &["redis"]),
    d("n8n-nodes-base.rabbitmq", "RabbitMQ", "transform", &[1.0, 1.1], "Sends messages to a RabbitMQ topic", 1, 1, &["operation", "queue", "options"], &["rabbitmq"]),
    d("n8n-nodes-base.kafka", "Kafka", "transform", &[1.0], "Sends messages to a Kafka topic", 1, 1, &["topic", "options"], &["kafka"]),
    d("n8n-nodes-base.mqtt", "MQTT", "input", &[1.0], "Push messages to MQTT", 1, 1, &["topic", "options"], &["mqtt"]),
    d("n8n-nodes-base.slack", "Slack", "output", &[2.0, 2.2, 2.3], "Consume Slack API", 1, 1, &["resource", "operation", "select", "text", "otherOptions"], &["slackApi", "slackOAuth2Api"]),
    d("n8n-nodes-base.googleSheets", "Google Sheets", "input", &[4.0, 4.5], "Read, update and write data to Google Sheets", 1, 1, &["resource", "operation", "documentId", "sheetName", "options"], &["googleSheetsOAuth2Api"]),
    d("n8n-nodes-base.gmail", "Gmail", "transform", &[2.0, 2.1], "Consume the Gmail API", 1, 1, &["resource", "operation", "sendTo", "subject", "message", "options"], &["gmailOAuth2"]),
    d("n8n-nodes-base.googleDrive", "Google Drive", "input", &[3.0], "Access data on Google Drive", 1, 1, &["resource", "operation", "fileId", "options"], &["googleDriveOAuth2Api"]),
    d("n8n-nodes-base.notion", "Notion", "output", &[2.0, 2.2], "Consume Notion API", 1, 1, &["resource", "operation", "databaseId", "options"], &["notionApi"]),
    d("n8n-nodes-base.airtable", "Airtable", "input", &[2.0, 2.1], "Read, update, write and delete data from Airtable", 1, 1, &["resource", "operation", "base", "table", "options"], &["airtableTokenApi"]),
    d("n8n-nodes-base.github", "GitHub", "input", &[1.0, 1.1], "Consume GitHub API", 1, 1, &["resource", "operation", "owner", "repository"], &["githubApi", "githubOAuth2Api"]),
    d("n8n-nodes-base.telegram", "Telegram", "output", &[1.0, 1.2], "Sends data to Telegram", 1, 1, &["resource", "operation", "chatId", "text", "additionalFields"], &["telegramApi"]),
    d("n8n-nodes-base.discord", "Discord", "output", &[2.0], "Sends data to Discord", 1, 1, &["resource", "operation", "content", "options"], &["discordBotApi", "discordWebhookApi"]),
    d("@n8n/n8n-nodes-langchain.openAi", "OpenAI", "transform", &[1.0, 1.8], "Message an assistant or GPT, analyze images, generate audio, etc.", 1, 1, &["resource", "operation", "modelId", "messages", "options"], &["openAiApi"]),
    d("@n8n/n8n-nodes-langchain.agent", "AI Agent", "transform", &[1.0, 1.9, 2.0], "Generates an action plan and executes it. Can use external tools.", 1, 1, &["promptType", "text", "options"], &[]),
    d("@n8n/n8n-nodes-langchain.lmChatOpenAi", "OpenAI Chat Model", "transform", &[1.0, 1.2], "For advanced usage with an AI chain", 0, 1, &["model", "options"], &["openAiApi"]),
];

fn describe(desc: &Desc) -> Value {
    let properties: Vec<Value> = desc.params.iter().map(|p| json!({"displayName": p, "name": p, "type": "string", "default": ""})).collect();
    let inputs = vec![json!("main"); desc.inputs];
    let outputs = vec![json!("main"); desc.outputs];
    let credentials: Vec<Value> = desc.credentials.iter().map(|c| json!({"name": c, "required": false})).collect();
    let version: Value = if desc.versions.len() == 1 { json!(desc.versions[0]) } else { json!(desc.versions) };
    json!({
        "name": desc.name,
        "displayName": desc.display,
        "group": [desc.group],
        "version": version,
        "description": desc.description,
        "defaults": {"name": desc.display},
        "inputs": inputs,
        "outputs": outputs,
        "properties": properties,
        "credentials": credentials,
    })
}

/// Descriptions of every allowed node type: the native ones, plus the GA
/// catalog entries r8r can show but not run yet (marked in `codex`).
pub fn descriptions(registry: &Registry, excluded: &[String]) -> Vec<Value> {
    DESCRIPTIONS
        .iter()
        .filter(|d| !excluded.iter().any(|e| e == d.name))
        .map(|d| {
            let mut v = describe(d);
            if registry.get(d.name).is_none() {
                v["codex"] = json!({"r8r": {"native": false}});
            }
            v
        })
        .collect()
}
