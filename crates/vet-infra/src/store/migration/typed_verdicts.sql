-- The vote staged and a platform's verdict the viewer voted (in a kept details copy) were
-- spelled as the platforms' ids (2.7x). The verdicts are typed now, and Vet keeps them spelled
-- its own way; what each was spelled becomes.

UPDATE changeset SET vote = CASE vote
    WHEN 'approved' THEN 'approve'
    WHEN 'approved-with-suggestions' THEN 'approve-with-suggestions'
    WHEN 'rejected' THEN 'reject'
    WHEN 'wait-for-author' THEN 'wait-for-author'
    WHEN 'COMMENT' THEN 'comment'
    WHEN 'APPROVE' THEN 'approve'
    WHEN 'REQUEST_CHANGES' THEN 'request-changes'
    ELSE vote
END
WHERE vote IS NOT NULL;

UPDATE pull_request SET detail = REPLACE(
    REPLACE(
        REPLACE(detail,
            '"viewer_verdict":"approved-with-suggestions"',
            '"viewer_verdict":"approve-with-suggestions"'),
        '"viewer_verdict":"approved"',
        '"viewer_verdict":"approve"'),
    '"viewer_verdict":"rejected"',
    '"viewer_verdict":"reject"');
