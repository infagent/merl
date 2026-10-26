use crate::{LaneError, MailSink, Store, failure, protocol};
use merl_mail::{Key, MAX_BODY_BYTES};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use std::fmt::Write;

// Five PR pointers fit the mail body's 8192-byte ceiling even with long valid
// repository names and every selected event type. More work forms another batch.
const PRS_PER_BATCH: i64 = 5;
const BATCHES_PER_PASS: usize = 16;

#[derive(Debug)]
struct Frozen {
    id: Key,
    recipient: Key,
    body: String,
}

impl Store {
    pub(super) fn flush(&mut self, mail: &mut impl MailSink, now: i64) -> Result<usize, LaneError> {
        let mut delivered = 0;
        for _ in 0..BATCHES_PER_PASS {
            self.freeze()?;
            let Some(batch) = self.claim()? else {
                break;
            };
            if let Err(error) = mail.publish(&batch.id, &batch.recipient, &batch.body, now) {
                self.db.execute(
                    "UPDATE outbox SET last_error=?3 WHERE id=?1 AND body=?2",
                    params![batch.id.as_str(), batch.body, error.code],
                )?;
                return Err(error);
            }
            // IDs and bodies are immutable. An older send acknowledgment can
            // finalize only its own frozen row, never newer staged activity.
            self.db.execute(
                "UPDATE outbox SET state='sent',last_error=NULL WHERE id=?1 AND body=?2",
                params![batch.id.as_str(), batch.body],
            )?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn freeze(&mut self) -> Result<(), LaneError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let first=tx.query_row("SELECT p.lane,p.generation,l.repo,l.agent,l.next_batch,l.gaps FROM pending p JOIN lanes l ON l.name=p.lane JOIN prs r ON r.lane=p.lane AND r.number=p.number WHERE l.paused=0 AND r.initializing=0 ORDER BY p.lane,p.generation,p.number LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).optional()?;
        let Some((lane, generation, repo, agent, prior, gaps)) = first else {
            return Ok(());
        };
        let sequence = prior
            .checked_add(1)
            .ok_or_else(|| failure("LANE_LIMIT", "lane mail sequence exhausted"))?;
        let id = format!("lane.{lane}.{sequence}");
        let rows = {
            let mut query=tx.prepare("SELECT p.number,p.head,p.state,p.reasons FROM pending p JOIN prs r ON r.lane=p.lane AND r.number=p.number WHERE p.lane=?1 AND p.generation=?2 AND r.initializing=0 ORDER BY p.number LIMIT ?3")?;
            query
                .query_map(params![lane, generation, PRS_PER_BATCH], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut body = format!(
            "Reviewer lane {lane}; observed configuration generation {generation}; discovery gaps {gaps}.\nFetch authoritative current PR state. Receipt does not complete review work.\n"
        );
        for (number, head, state, reasons) in &rows {
            let reasons: Vec<Value> = serde_json::from_str(reasons).map_err(|_| protocol())?;
            let reasons = reasons
                .iter()
                .map(|v| v.as_str().ok_or_else(protocol))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            writeln!(
                body,
                "https://github.com/{repo}/pull/{number} head {head} state {state}: {reasons}"
            )
            .map_err(|_| protocol())?;
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(failure(
                "LANE_LIMIT",
                "bounded activity batch exceeds mail limit; intent retained",
            ));
        }
        tx.execute("INSERT INTO outbox(lane,sequence,id,recipient,body,generation) VALUES(?1,?2,?3,?4,?5,?6)",params![lane,sequence,id,agent,body,generation])?;
        tx.execute(
            "UPDATE lanes SET next_batch=?2 WHERE name=?1",
            params![lane, sequence],
        )?;
        for (number, _, _, _) in rows {
            tx.execute(
                "DELETE FROM pending WHERE lane=?1 AND number=?2 AND generation=?3",
                params![lane, number, generation],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn claim(&mut self) -> Result<Option<Frozen>, LaneError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT o.id,o.recipient,o.body FROM outbox o JOIN lanes l ON l.name=o.lane WHERE l.paused=0 AND o.state!='sent' ORDER BY o.lane,o.sequence LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        let batch = row
            .map(|(id, recipient, body)| -> Result<Frozen, LaneError> {
                Ok(Frozen {
                    id: id.parse().map_err(|_| protocol())?,
                    recipient: recipient.parse().map_err(|_| protocol())?,
                    body,
                })
            })
            .transpose()?;
        if let Some(batch) = &batch {
            tx.execute(
                "UPDATE outbox SET state='sending' WHERE id=?1",
                [batch.id.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(batch)
    }
}
