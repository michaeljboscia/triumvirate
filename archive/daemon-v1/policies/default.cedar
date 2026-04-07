permit(
    principal == User::"human",
    action in [Action::"fleet_merge", Action::"git_push", Action::"file_delete", Action::"db_drop"],
    resource
);

forbid(
    principal != User::"human",
    action in [Action::"fleet_merge", Action::"git_push", Action::"file_delete", Action::"db_drop"],
    resource
);
