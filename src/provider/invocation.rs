use crate::provider::{
    resolve_anthropic_model, InvocationSpec, ProviderError, ProviderKind, ProviderRequest,
    RunnerBinaries,
};

pub fn build_invocation(
    request: &ProviderRequest,
    binaries: &RunnerBinaries,
) -> Result<InvocationSpec, ProviderError> {
    match request.provider {
        ProviderKind::Anthropic => {
            let model = resolve_anthropic_model(&request.model)?;
            let mut args = vec!["--dangerously-skip-permissions".to_string()];
            args.push("--model".to_string());
            args.push(model.clone());
            if !request.reset_requested && !request.fresh_on_failure {
                args.push("-c".to_string());
            }
            args.push("-p".to_string());
            args.push(request.message.clone());
            Ok(InvocationSpec {
                binary: binaries.anthropic.clone(),
                args,
                resolved_model: model,
            })
        }
        ProviderKind::OpenAi => {
            let mut args = vec!["exec".to_string()];
            if !request.reset_requested && !request.fresh_on_failure {
                args.push("resume".to_string());
                args.push("--last".to_string());
            }
            args.push("--model".to_string());
            args.push(request.model.clone());
            args.push("--skip-git-repo-check".to_string());
            args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
            args.push("--json".to_string());
            args.push(request.message.clone());
            Ok(InvocationSpec {
                binary: binaries.openai.clone(),
                args,
                resolved_model: request.model.clone(),
            })
        }
    }
}
