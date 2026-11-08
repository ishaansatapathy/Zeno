create type enrollment_source as enum ('payment', 'free', 'grant', 'admin');

alter table enrollments
    alter column payment_id drop not null,
    add column source enrollment_source not null default 'payment',
    add column granted_by uuid references users(id) on delete set null,
    add column revoked_at timestamptz,
    add column revoke_reason text,
    add column updated_at timestamptz not null default now(),
    add constraint enrollments_payment_source_check check (
        source <> 'payment' or payment_id is not null
    ),
    add constraint enrollments_revoke_state_check check (
        (revoked_at is null and revoke_reason is null)
        or
        (revoked_at is not null and char_length(revoke_reason) between 1 and 300)
    );

create index enrollments_user_active_idx
    on enrollments (user_id, course_id)
    where revoked_at is null;
create index enrollments_user_list_idx
    on enrollments (user_id, created_at desc, id desc);
create index enrollments_course_list_idx
    on enrollments (course_id, created_at desc, id desc);
create index lessons_video_access_idx
    on lessons (video_id, course_id)
    where deleted_at is null and video_id is not null;

-- An immutable entitlement trail: the enrollment row is re-used on reactivation, while this
-- table preserves who granted/revoked it and when.
create table enrollment_events (
    id uuid primary key default uuid_generate_v4(),
    enrollment_id uuid not null references enrollments(id) on delete cascade,
    actor_id uuid references users(id) on delete set null,
    action text not null check (action in ('created', 'reactivated', 'revoked')),
    source enrollment_source not null,
    reason text,
    created_at timestamptz not null default now()
);

create index enrollment_events_enrollment_idx
    on enrollment_events (enrollment_id, created_at desc);

-- Invitations are bound to a verified email. Only a SHA-256 token hash is retained; verification
-- of the same mailbox converts every live invite atomically into an enrollment.
create table course_invites (
    id uuid primary key default uuid_generate_v4(),
    course_id uuid not null references courses(id),
    email text not null,
    token_hash bytea not null unique,
    granted_by uuid not null references users(id),
    access_expires_at timestamptz,
    token_expires_at timestamptz not null,
    accepted_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    constraint course_invites_email_check check (
        email = lower(email) and char_length(email) between 3 and 254
    ),
    constraint course_invites_state_check check (
        accepted_at is null or revoked_at is null
    )
);

create unique index course_invites_pending_unique
    on course_invites (course_id, email)
    where accepted_at is null and revoked_at is null;
create index course_invites_email_pending_idx
    on course_invites (email, token_expires_at)
    where accepted_at is null and revoked_at is null;

alter table playback_sessions
    drop constraint playback_sessions_end_reason_check,
    add constraint playback_sessions_end_reason_check check (
        end_reason in (
            'stopped', 'idle', 'expired', 'password_reset', 'video_deleted', 'access_revoked'
        )
    );

-- Single source of truth for video access. Owner/admin access is course-independent. Free previews
-- require a published live course; active enrollments keep working while a course is being edited,
-- and for the promised 30-day grace period after soft deletion.
create function video_access_state(p_user_id uuid, p_video_id uuid)
returns text
language sql
stable
security invoker
set search_path = public
as $$
    select case
        when exists (
            select 1 from videos v join users u on u.id = p_user_id
            where v.id = p_video_id and (v.owner_id = p_user_id or u.role = 'admin')
        ) then 'allowed'
        when exists (
            select 1
            from lessons l
            join courses c on c.id = l.course_id
            where l.video_id = p_video_id
              and l.deleted_at is null
              and (
                  (
                      l.is_free_preview
                      and c.status = 'published'
                      and c.deleted_at is null
                  )
                  or exists (
                      select 1 from enrollments e
                      where e.course_id = c.id
                        and e.user_id = p_user_id
                        and e.revoked_at is null
                        and (e.expires_at is null or e.expires_at > now())
                        and (c.deleted_at is null or c.deleted_at > now() - interval '30 days')
                  )
              )
        ) then 'allowed'
        when exists (
            select 1
            from lessons l
            join courses c on c.id = l.course_id
            join enrollments e on e.course_id = c.id and e.user_id = p_user_id
            where l.video_id = p_video_id
              and l.deleted_at is null
              and e.revoked_at is null
              and e.expires_at <= now()
              and (c.deleted_at is null or c.deleted_at > now() - interval '30 days')
        ) then 'expired'
        else 'access_revoked'
    end
$$;
