const MODEL_CONTEXT_ATTACHMENT_KINDS = new Set([
    'skill_listing',
    'invoked_skills',
    'running_agents',
    'todo_list',
    'plan_mode',
    'hook_additional_context',
]);
export function isModelContextAttachmentMessage(message) {
    const kind = message.attachment_metadata?.kind;
    return typeof kind === 'string' && MODEL_CONTEXT_ATTACHMENT_KINDS.has(kind);
}
