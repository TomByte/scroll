//! Where the palette's rows come from: recently visited pages, the search index, tags, the
//! pages carrying a tag, and the answers a command asks for.

use super::*;

impl CommandPalette {
    /// Instant results: commands, tags and (with nothing typed) recent pages.
    pub(super) fn search_items(&self, query: &str, cx: &App) -> Vec<Item> {
        let (mode, words) = mode_of(query);
        let mut items = Vec::new();
        if matches!(mode, Mode::Everything | Mode::Pages) && words.is_empty() {
            items.extend(self.recent_page_items(cx));
        }
        if matches!(mode, Mode::Everything | Mode::Commands)
            && (mode == Mode::Commands || !words.is_empty())
        {
            items.extend(
                matching_commands(self.commands, words, &self.situation, &self.usage)
                    .into_iter()
                    .take(SECTION_LIMIT)
                    .map(Item::Command),
            );
        }
        if matches!(mode, Mode::Everything | Mode::Tags)
            && (mode == Mode::Tags || !words.is_empty())
        {
            items.extend(self.tag_items(words, cx));
        }
        items
    }

    pub(super) fn recent_page_items(&self, cx: &App) -> Vec<Item> {
        let tree = self.workspace.read(cx).tree();
        self.recent_pages
            .iter()
            .filter(|id| tree.is_live(**id))
            .filter_map(|id| {
                let page = tree.page(*id)?;
                Some(Item::Page {
                    id: *id,
                    title: labelled_title(page).into(),
                    snippet: None,
                })
            })
            .take(RECENT_LIMIT)
            .collect()
    }

    pub(super) fn tag_items(&self, words: &str, cx: &App) -> Vec<Item> {
        let mut ranked: Vec<(u32, Item)> = self
            .workspace
            .read(cx)
            .records()
            .tags_with_counts()
            .into_iter()
            .filter_map(|(tag, count)| {
                let score = fuzzy_score(words, &tag.name)?;
                Some((
                    score,
                    Item::Tag {
                        id: tag.id,
                        name: tag.name.clone().into(),
                        count,
                    },
                ))
            })
            .collect();
        ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        ranked
            .into_iter()
            .take(SECTION_LIMIT)
            .map(|(_, item)| item)
            .collect()
    }

    pub(super) fn choice_items(&self, command: &Command, query: &str, cx: &App) -> Vec<Item> {
        let query = query.trim();
        let Run::Choose { choices, .. } = &command.run else {
            return Vec::new();
        };
        match choices {
            Choices::Fixed(_) => {
                let mut ranked: Vec<(u32, Choice)> = self
                    .fixed_choices
                    .iter()
                    .cloned()
                    .filter_map(|choice| Some((fuzzy_score(query, &choice.label)?, choice)))
                    .collect();
                ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                ranked
                    .into_iter()
                    .map(|(_, choice)| Item::Choice(choice))
                    .collect()
            }
            Choices::Pages => {
                let mut items = Vec::new();
                if fuzzy_score(query, "Top level").is_some() {
                    items.push(Item::Choice(Choice {
                        label: "Top level of the workspace".into(),
                        emoji: None,
                        value: ChoiceValue::Page(None),
                    }));
                }
                items.extend(self.recent_page_items(cx).into_iter().filter_map(
                    |item| match item {
                        Item::Page { id, title, .. } if fuzzy_score(query, &title).is_some() => {
                            Some(Item::Choice(Choice {
                                label: title,
                                emoji: None,
                                value: ChoiceValue::Page(Some(id)),
                            }))
                        }
                        _ => None,
                    },
                ));
                items
            }
            Choices::Tags => {
                let records = self.workspace.read(cx).records();
                let mut items: Vec<Item> = self
                    .tag_items(query, cx)
                    .into_iter()
                    .filter_map(|item| match item {
                        Item::Tag { name, .. } => Some(Item::Choice(Choice {
                            label: name.clone(),
                            emoji: None,
                            value: ChoiceValue::TagName(name.to_string()),
                        })),
                        _ => None,
                    })
                    .collect();
                if !query.is_empty() && records.tag_named(query).is_none() {
                    items.insert(
                        0,
                        Item::Choice(Choice {
                            label: format!("Create tag “{query}”").into(),
                            emoji: None,
                            value: ChoiceValue::TagName(query.to_string()),
                        }),
                    );
                }
                items
            }
        }
    }

    /// The words to search the index for, if `query` lists pages from it.
    pub(super) fn page_search_words(&self, query: &str) -> Option<String> {
        let words = match &self.step {
            Step::Search => match mode_of(query) {
                (Mode::Everything | Mode::Pages, words) => words,
                _ => return None,
            },
            Step::Choose { command, .. }
                if matches!(
                    command.run,
                    Run::Choose {
                        choices: Choices::Pages,
                        ..
                    }
                ) =>
            {
                query.trim()
            }
            _ => return None,
        };
        (!words.is_empty()).then(|| words.to_string())
    }

    /// Asks the search index for pages on the storage thread; the answer joins the pending
    /// results and replaces the list, if the query has not changed meanwhile.
    pub(super) fn search_pages_in_background(
        &mut self,
        words: String,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let typed = std::time::Instant::now();
        let storage = self.storage.clone();
        self.search = cx.spawn(async move |palette, cx| {
            // A pasted or fast-typed query replaces this task before the pause ends, so only
            // the last prefix is searched instead of one search per keystroke.
            cx.background_executor().timer(SEARCH_PAUSE).await;
            let searching = match cx.update(|cx| {
                let words = words.clone();
                storage.read(cx, move |store| storage::search(store, &words, PAGE_LIMIT))
            }) {
                Ok(searching) => searching,
                Err(error) => return log::debug!("palette closed before searching: {error}"),
            };
            // A failed search still shows the query's other results.
            let hits = searching.await.unwrap_or_else(|error| {
                log::warn!("search failed: {error}");
                Vec::new()
            });
            let shown = palette.update(cx, |palette, cx| {
                if palette.query(cx) != query {
                    return;
                }
                let Some(items) = palette.pending_items.take() else {
                    return;
                };
                let count = hits.len();
                palette.show(items, cx);
                palette.add_page_hits(hits, &words, cx);
                log::debug!("timing: palette {query:?} showed {count} page results {:?} after the keystroke", typed.elapsed());
            });
            if let Err(error) = shown {
                log::debug!("palette closed before search finished: {error}");
            }
        });
    }

    /// Ranks hits by how well the title matches and how recently the page was open, then puts
    /// them first (pages lead in plain search) or as choices.
    pub(super) fn add_page_hits(
        &mut self,
        hits: Vec<scroll_core::store::SearchHit>,
        words: &str,
        cx: &App,
    ) {
        let tree = self.workspace.read(cx).tree();
        let mut ranked: Vec<(u32, scroll_core::store::SearchHit)> = hits
            .into_iter()
            .filter(|hit| {
                tree.page(hit.page_id)
                    .is_none_or(|page| page.row_of.is_none() || !self.is_choosing())
            })
            .map(|hit| {
                let title_score = fuzzy_score(words, &hit.title).unwrap_or(0);
                let recency = self
                    .recent_pages
                    .iter()
                    .position(|recent| *recent == hit.page_id)
                    .map_or(0, |position| 20u32.saturating_sub(position as u32 * 2));
                (title_score + recency, hit)
            })
            .collect();
        ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        let pages: Vec<Item> = ranked
            .into_iter()
            .filter(|(_, hit)| {
                !self.items.iter().any(|item| match item {
                    Item::Page { id, .. } => *id == hit.page_id,
                    Item::Choice(Choice {
                        value: ChoiceValue::Page(Some(id)),
                        ..
                    }) => *id == hit.page_id,
                    _ => false,
                })
            })
            .map(|(_, hit)| {
                let title: SharedString = tree
                    .page(hit.page_id)
                    .map_or_else(|| hit.title.clone(), labelled_title)
                    .into();
                if self.is_choosing() {
                    Item::Choice(Choice {
                        label: title,
                        emoji: None,
                        value: ChoiceValue::Page(Some(hit.page_id)),
                    })
                } else {
                    let snippet = (hit.snippet != hit.title).then(|| hit.snippet.into());
                    Item::Page {
                        id: hit.page_id,
                        title,
                        snippet,
                    }
                }
            })
            .collect();
        let insert_at = if self.is_choosing() {
            self.items.len()
        } else {
            0
        };
        self.items.splice(insert_at..insert_at, pages);
    }

    /// Reads the tag's pages on the storage thread, at most `TAGGED_LIMIT` of them.
    pub(super) fn load_tagged_pages(&mut self, tag: TagId, cx: &mut Context<Self>) {
        let reading = self.storage.read(cx, move |store| {
            Ok(store.pages_with_tags(&[tag], TAGGED_LIMIT)?)
        });
        self.tag_load = cx.spawn(async move |palette, cx| {
            let pages = match reading.await {
                Ok(pages) => pages,
                Err(error) => return log::error!("could not read tagged pages: {error}"),
            };
            let shown = palette.update(cx, |palette, cx| {
                if let Step::TaggedPages(loaded) = &mut palette.step {
                    *loaded = pages;
                    palette.query_changed(cx);
                }
            });
            if shown.is_err() {
                log::debug!("palette closed before tagged pages arrived");
            }
        });
    }
}

/// Tagged pages whose title matches what is typed.
pub(super) fn tagged_page_items(pages: &[scroll_core::record::Page], query: &str) -> Vec<Item> {
    pages
        .iter()
        .filter(|page| fuzzy_score(query.trim(), &page.title).is_some())
        .map(|page| Item::Page {
            id: page.id,
            title: labelled_title(page).into(),
            snippet: None,
        })
        .collect()
}
