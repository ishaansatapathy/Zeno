-- Persist the policy independently of the actor FK: deleting an actor must not erase a block.
alter table enrollments
    add column revoked_by uuid references users(id) on delete set null,
    add column revocation_kind text check (
        revocation_kind in ('creator', 'admin', 'payment', 'system')
    );

-- Recover attribution for enrollments already revoked through the manual API.
update enrollments e
set revoked_by = event.actor_id,
    revocation_kind = case when actor.role = 'admin' then 'admin' else 'creator' end
from enrollment_events event
join users actor on actor.id = event.actor_id
where event.enrollment_id = e.id
  and event.action = 'revoked'
  and event.created_at = e.revoked_at
  and e.revoked_at is not null;

alter table enrollments add constraint enrollments_revocation_metadata_check check (
    revoked_at is not null or (revoked_by is null and revocation_kind is null)
);

-- Invitations are accepted through verified mailbox ownership. Retain the legacy hash column
-- for rolling compatibility, but new invitations no longer generate unused bearer tokens.
alter table course_invites alter column token_hash drop not null;
