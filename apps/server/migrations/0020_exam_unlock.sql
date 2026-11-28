-- Emergency unlock for exam rooms. The supervisor sees the code and reads it
-- to one student. The student computer checks it with the server, or, when
-- the server is out of reach, against this salted PBKDF2 hash it was given.
ALTER TABLE exam_rooms
    ADD COLUMN unlock_code TEXT,
    ADD COLUMN unlock_salt TEXT,
    ADD COLUMN unlock_hash TEXT;
