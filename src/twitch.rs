use http::StatusCode;
use lazy_static::lazy_static;
use log;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use std::result::Result;
use urlencoding;

use crate::PlaylistItem;

const GRAPHQL_URL: &str = "https://gql.twitch.tv/gql";

lazy_static! {
  // https://www.twitch.tv/speedgaming
  static ref CHANNEL_URL_PATTERNS: [Regex; 1] = [
    Regex::new(r"^https?://www\.twitch\.tv/(?P<channel_name>[^/?#]+)").unwrap(),
  ];

  // https://www.twitch.tv/videos/113837699
  // https://www.twitch.tv/gamesdonequick/video/113837699 (legacy url)
  // https://www.twitch.tv/gamesdonequick/v/113837699 (legacy url)
  // https://player.twitch.tv/?video=v113837699&parent=example.com ("v" is optional)
  static ref VIDEO_URL_PATTERNS: [Regex; 3] = [
    Regex::new(r"^https?://www\.twitch\.tv/videos/(?P<video_id>\d+)").unwrap(),
    Regex::new(r"^https?://www\.twitch\.tv/[^/]+/v(?:ideo)?/(?P<video_id>\d+)").unwrap(),
    Regex::new(r"^https?://player\.twitch\.tv/[^#]*[?&]video=v?(?P<video_id>\d+)").unwrap(),
  ];

  // https://clips.twitch.tv/AmazonianKnottyLapwingSwiftRage
  // https://www.twitch.tv/gamesdonequick/clip/ExuberantMiniatureSandpiperDogFace
  static ref CLIP_URL_PATTERNS: [Regex; 2] = [
    Regex::new(r"^https?://clips\.twitch\.tv/(?P<slug>[^/?#]+)").unwrap(),
    Regex::new(r"^https?://www\.twitch\.tv/[^/]+/clip/(?P<slug>[^/?#]+)").unwrap(),
  ];
}

#[derive(Debug)]
pub enum TwitchMatch {
  Channel(String),
  Video(String),
  Clip(String),
}

// Channel
#[derive(Debug, Deserialize)]
struct ChannelResponseData {
  data: ChannelData,
}

#[derive(Debug, Deserialize)]
struct ChannelData {
  channel: Option<Channel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Channel {
  display_name: Option<String>,
  stream: Option<Stream>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stream {
  title: String,
  created_at: String,
  language: String,
  game: Option<Game>,
  playback_access_token: PlaybackAccessToken,
}

// Video
#[derive(Debug, Deserialize)]
struct VideoResponseData {
  data: VideoData,
}

#[derive(Debug, Deserialize)]
struct VideoData {
  video: Option<Video>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Video {
  title: String,
  description: Option<String>,
  owner: User,
  game: Option<Game>,
  recorded_at: String,
  duration: String,
  language: String,
  playback_access_token: PlaybackAccessToken,
}

// Clip
#[derive(Debug, Deserialize)]
struct ClipResponseData {
  data: ClipData,
}

#[derive(Debug, Deserialize)]
struct ClipData {
  clip: Option<Clip>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Clip {
  title: String,
  broadcaster: User,
  game: Option<Game>,
  created_at: String,
  duration_seconds: usize,
  language: String,
  playback_access_token: PlaybackAccessToken,
}

#[derive(Debug, Deserialize)]
struct ClipTokenValue {
  clip_uri: String,
}

// Shared
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Game {
  display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
  display_name: String,
}

#[derive(Debug, Deserialize)]
struct PlaybackAccessToken {
  signature: String,
  value: String,
}

pub fn probe(url: &str) -> Option<TwitchMatch> {
  if crate::CONFIG.twitch_client_id.is_none() {
    return None;
  }

  for re in CLIP_URL_PATTERNS.iter() {
    if cfg!(debug_assertions) {
      log::info!("re: {:?}", re);
    }
    let ret = re.captures(url);
    if ret.is_some() {
      return Some(TwitchMatch::Clip(
        ret.unwrap().get(1).unwrap().as_str().to_string(),
      ));
    }
  }

  for re in VIDEO_URL_PATTERNS.iter() {
    if cfg!(debug_assertions) {
      log::info!("re: {:?}", re);
    }
    let ret = re.captures(url);
    if ret.is_some() {
      return Some(TwitchMatch::Video(
        ret.unwrap().get(1).unwrap().as_str().to_string(),
      ));
    }
  }

  for re in CHANNEL_URL_PATTERNS.iter() {
    if cfg!(debug_assertions) {
      log::info!("re: {:?}", re);
    }
    let ret = re.captures(url);
    if ret.is_some() {
      return Some(TwitchMatch::Channel(
        ret.unwrap().get(1).unwrap().as_str().to_lowercase(),
      ));
    }
  }

  return None;
}

pub async fn resolve(m: TwitchMatch) -> Result<Vec<PlaylistItem>, &'static str> {
  match m {
    TwitchMatch::Channel(channel_name) => resolve_channel(channel_name).await,
    TwitchMatch::Video(video_id) => resolve_video(video_id).await,
    TwitchMatch::Clip(slug) => resolve_clip(slug).await,
  }
}

async fn resolve_channel(channel_name: String) -> Result<Vec<PlaylistItem>, &'static str> {
  // https://www.twitch.tv/directory/game/Perfect%20Dark
  // https://www.twitch.tv/recaps/annual
  if channel_name == "directory" || channel_name == "recaps" {
    return Err("unsupported channel name");
  }

  let request_data = json!({
    "query": include_str!("twitch/channel.gql"),
    "variables": {
      "channelName": channel_name,
      "platform": "web",
      "playerType": "site",
    },
  });

  let client = reqwest::Client::builder()
    .build()
    .expect("build reqwest client");
  let client_id = crate::CONFIG.twitch_client_id.as_ref().unwrap().as_str();
  let response = client
    .post(GRAPHQL_URL)
    .header("Client-ID", client_id)
    .body(serde_json::to_string(&request_data).unwrap())
    .send()
    .await
    .expect("send graphql request");
  let response_status = response.status();
  let response_text = response.text().await.expect("read response data");

  if response_status != (StatusCode::OK) {
    log::error!("bad response: {} - {:?}", response_status, response_text);
    return Err("received non-200 response from Twitch");
  }

  let response_data: ChannelResponseData = match serde_json::from_str(response_text.as_str()) {
    Ok(v) => v,
    Err(e) => {
      log::error!("error: {:?}", e);
      if cfg!(debug_assertions) {
        log::info!("response_text: {}", response_text);
      }
      return Err("error deserializing data");
    }
  };
  if cfg!(debug_assertions) {
    log::info!("response_data: {:?}", response_data);
  }
  if response_data.data.channel.is_none() {
    return Err("channel does not exist");
  }
  let channel = response_data.data.channel.unwrap();
  if channel.stream.is_none() {
    return Err("channel is not live");
  }
  let stream = channel.stream.unwrap();

  return Ok(vec![PlaylistItem {
    path: format!(
      "https://usher.ttvnw.net/api/channel/hls/{}.m3u8?allow_source=true&allow_audio_only=true&sig={}&token={}",
      channel_name,
      urlencoding::encode(stream.playback_access_token.signature.as_str()),
      urlencoding::encode(stream.playback_access_token.value.as_str())
    ),
    name: stream.title,
    description: None,
    artist: channel.display_name,
    genre: stream.game.and_then(|game| Some(game.display_name)),
    date: Some(stream.created_at.replace("T", " ").replace("Z", "")),
    duration: None,
    language: Some(stream.language),
  }]);
}

async fn resolve_video(video_id: String) -> Result<Vec<PlaylistItem>, &'static str> {
  let q = json!({
    "query": include_str!("twitch/video.gql"),
    "variables": {
      "vodID": video_id,
      "platform": "web",
      "playerType": "site",
    },
  });
  let request_data = serde_json::to_string(&q).unwrap();

  let client = reqwest::Client::builder()
    .build()
    .expect("build reqwest client");
  let client_id = crate::CONFIG.twitch_client_id.as_ref().unwrap().as_str();
  let response = client
    .post(GRAPHQL_URL)
    .header("Client-ID", client_id)
    .body(request_data)
    .send()
    .await
    .expect("send graphql request");
  let response_status = response.status();
  let response_text = response.text().await.expect("read response data");

  if response_status != (StatusCode::OK) {
    log::error!("bad response: {} - {:?}", response_status, response_text);
    return Err("received non-200 response from Twitch");
  }

  let response_data: VideoResponseData = match serde_json::from_str(response_text.as_str()) {
    Ok(v) => v,
    Err(e) => {
      log::error!("error: {:?}", e);
      if cfg!(debug_assertions) {
        log::info!("response_text: {}", response_text);
      }
      return Err("error deserializing data");
    }
  };
  if cfg!(debug_assertions) {
    log::info!("response_data: {:?}", response_data);
  }
  if response_data.data.video.is_none() {
    return Err("video is null");
  }
  let video = response_data.data.video.unwrap();

  return Ok(vec![PlaylistItem {
    path: format!(
      "https://usher.ttvnw.net/vod/{}.m3u8?allow_source=true&allow_audio_only=true&sig={}&token={}",
      video_id,
      urlencoding::encode(video.playback_access_token.signature.as_str()),
      urlencoding::encode(video.playback_access_token.value.as_str())
    ),
    name: video.title,
    description: video.description,
    artist: Some(video.owner.display_name),
    genre: video.game.and_then(|game| Some(game.display_name)),
    date: Some(video.recorded_at.replace("T", " ").replace("Z", "")),
    duration: Some(parse_duration(video.duration.as_str())),
    language: Some(video.language),
  }]);
}

async fn resolve_clip(slug: String) -> Result<Vec<PlaylistItem>, &'static str> {
  let q = json!({
    "query": include_str!("twitch/clip.gql"),
    "variables": {
      "slug": slug,
      "platform": "web",
      "playerType": "site",
    },
  });
  let request_data = serde_json::to_string(&q).unwrap();

  let client = reqwest::Client::builder()
    .build()
    .expect("build reqwest client");
  let client_id = crate::CONFIG.twitch_client_id.as_ref().unwrap().as_str();
  let response = client
    .post(GRAPHQL_URL)
    .header("Client-ID", client_id)
    .body(request_data)
    .send()
    .await
    .expect("send graphql request");
  let response_status = response.status();
  let response_text = response.text().await.expect("read response data");

  if response_status != (StatusCode::OK) {
    log::error!("bad response: {} - {:?}", response_status, response_text);
    return Err("received non-200 response from Twitch");
  }

  let response_data: ClipResponseData = match serde_json::from_str(response_text.as_str()) {
    Ok(v) => v,
    Err(e) => {
      log::error!("error: {:?}", e);
      if cfg!(debug_assertions) {
        log::info!("response_text: {}", response_text);
      }
      return Err("error deserializing data");
    }
  };
  if cfg!(debug_assertions) {
    log::info!("response_data: {:?}", response_data);
  }
  if response_data.data.clip.is_none() {
    return Err("clip is null");
  }
  let clip = response_data.data.clip.unwrap();
  let token_value: ClipTokenValue =
    match serde_json::from_str(clip.playback_access_token.value.as_str()) {
      Ok(v) => v,
      Err(e) => {
        log::error!("error: {:?}", e);
        return Err("error deserializing token_value");
      }
    };
  if cfg!(debug_assertions) {
    log::info!("token_value: {:?}", token_value);
  }

  return Ok(vec![PlaylistItem {
    path: format!(
      "{}?allow_source=true&allow_audio_only=true&sig={}&token={}",
      token_value.clip_uri,
      urlencoding::encode(clip.playback_access_token.signature.as_str()),
      urlencoding::encode(clip.playback_access_token.value.as_str())
    ),
    name: clip.title,
    description: None,
    artist: Some(clip.broadcaster.display_name),
    genre: clip.game.and_then(|game| Some(game.display_name)),
    date: Some(clip.created_at.replace("T", " ").replace("Z", "")),
    duration: Some(clip.duration_seconds),
    language: Some(clip.language),
  }]);
}

fn parse_duration(s: &str) -> usize {
  let mut seconds = 0;
  let mut numbers = String::with_capacity(2);
  for c in s.chars() {
    if c.is_ascii_digit() {
      numbers.push(c);
    } else if c == 'h' || c == 'm' || c == 's' {
      match numbers.parse::<usize>() {
        Ok(n) => {
          if n != 0 {
            if c == 'h' {
              seconds += 3600 * n;
            } else if c == 'm' {
              seconds += 60 * n;
            } else if c == 's' {
              seconds += n;
            }
          }
        }
        Err(e) => {
          log::error!("parse_duration({}) error: {}", s, e);
        }
      }
      numbers.clear();
    } else {
      log::error!("parse_duration({}) unexpected character: {}", s, c);
    }
  }
  return seconds;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test() {
    // Durations taken from Twitch:
    assert_eq!(parse_duration("32h47m50s"), 118070);
    assert_eq!(parse_duration("1h20m0s"), 4800);
    assert_eq!(parse_duration("55m31s"), 3331);
    assert_eq!(parse_duration("2m53s"), 173);
    assert_eq!(parse_duration("58s"), 58);

    // Hypothetical durations that shouldn't error:
    assert_eq!(parse_duration("1h"), 3600);
    assert_eq!(parse_duration("1m"), 60);
    assert_eq!(parse_duration("0s"), 0);

    // Not seen on Twitch so they don't work properly:
    assert_eq!(parse_duration("1d8h47m50s"), 67670);
    assert_eq!(parse_duration("1y10d"), 0);
  }
}
