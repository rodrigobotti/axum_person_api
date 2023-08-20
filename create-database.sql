-- ideally we would use migrations
-- but alas I'm lazy and did it this way
-- specially since this is not supposed to be a "production grade" project

CREATE TABLE person (
    id BIGSERIAL PRIMARY KEY,
    nickname VARCHAR NOT NULL UNIQUE,
    "name" VARCHAR NOT NULL,
    dob DATE NOT NULL,
    stacks VARCHAR[]
);

-- TODO: search indexes