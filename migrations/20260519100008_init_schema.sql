-- Add migration script here
CREATE TABLE Users (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  username VARCHAR(50) NOT NULL UNIQUE,
  password VARCHAR(255) NOT NULL,
  email VARCHAR(100) NOT NULL UNIQUE,
  profile_pic VARCHAR(255),
  dob DATE,
  phone_number VARCHAR(20),
  created_at TIMESTAMP DEFAULT NOW(),
  status VARCHAR(20) DEFAULT 'active',
  role VARCHAR(20) DEFAULT 'user',
  follower_count INT DEFAULT 0,
  following_count INT DEFAULT 0
);

CREATE TABLE User_detail (
  user_id UUID PRIMARY KEY REFERENCES Users(id) ON DELETE CASCADE,
  tech_stack TEXT,
  projects TEXT,
  experience TEXT,
  certificates TEXT,
  cv TEXT,
  roles TEXT
);

CREATE TABLE Posts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  caption TEXT,
  video_url VARCHAR(255),
  thumbnail_url VARCHAR(255),
  created_at TIMESTAMP DEFAULT NOW(),
  visibility VARCHAR(20) DEFAULT 'public'
);

CREATE TABLE User_Follows (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  follower_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  following_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  created_at TIMESTAMP DEFAULT NOW(),
  UNIQUE (follower_id, following_id)
);

CREATE TABLE Likes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  post_id UUID NOT NULL REFERENCES Posts(id) ON DELETE CASCADE,
  created_at TIMESTAMP DEFAULT NOW(),
  UNIQUE (user_id, post_id)
);

CREATE TABLE Comments (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  post_id UUID NOT NULL REFERENCES Posts(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  parent_id UUID REFERENCES Comments(id) ON DELETE CASCADE,
  created_at TIMESTAMP DEFAULT NOW()
);

CREATE TABLE Jobs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  title VARCHAR(100) NOT NULL,
  location VARCHAR(100),
  created_at TIMESTAMP DEFAULT NOW(),
  company_name VARCHAR(100),
  experience_required VARCHAR(50)
);

CREATE TABLE Jobs_detail (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  job_id UUID NOT NULL REFERENCES Jobs(id) ON DELETE CASCADE,
  description TEXT,
  requirements TEXT,
  company_name VARCHAR(100),
  package VARCHAR(50),
  location VARCHAR(100),
  type VARCHAR(50)
);

CREATE TABLE Conversations (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  type VARCHAR(10) NOT NULL,
  name VARCHAR(100),
  created_at TIMESTAMP DEFAULT NOW(),
  last_message_at TIMESTAMP
);

CREATE TABLE Conversation_Participants (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  conversation_id UUID NOT NULL REFERENCES Conversations(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  last_read_at TIMESTAMP,
  role VARCHAR(10) DEFAULT 'member',
  joined_at TIMESTAMP DEFAULT NOW(),
  UNIQUE (conversation_id, user_id)
);

CREATE TABLE Messages (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  conversation_id UUID NOT NULL REFERENCES Conversations(id) ON DELETE CASCADE,
  sender_id UUID NOT NULL REFERENCES Users(id) ON DELETE CASCADE,
  content TEXT,
  message_type VARCHAR(10) NOT NULL DEFAULT 'text',
  media_url VARCHAR(255),
  shared_post_id UUID REFERENCES Posts(id) ON DELETE SET NULL,
  shared_job_id UUID REFERENCES Jobs(id) ON DELETE SET NULL,
  reply_to_id UUID REFERENCES Messages(id) ON DELETE SET NULL,
  is_deleted BOOLEAN DEFAULT FALSE,
  created_at TIMESTAMP DEFAULT NOW()
);
